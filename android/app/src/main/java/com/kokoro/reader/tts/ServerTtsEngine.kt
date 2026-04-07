package com.kokoro.reader.tts

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import kotlinx.coroutines.*
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

data class ServerVoice(val id: String, val name: String, val lang: String)

/**
 * Continuous TTS session: speak() starts, appendPage() feeds more pages,
 * finishSession() signals end. Audio plays across pages without interruption.
 */
class ServerTtsEngine(private var serverUrl: String) {
    var state: TtsState = TtsState.IDLE
        private set
    var currentSentence: Int = 0
        private set
    var totalSentences: Int = 0
        private set

    private var sentences: MutableList<String> = mutableListOf()
    private val generationId = AtomicLong(0)
    @Volatile private var paused = false
    private var speakJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    // Sentence queue for the worker — END_MARKER signals session end
    data class SentenceJob(val sentence: String, val globalIdx: Int, val isPageBoundary: Boolean, val isEnd: Boolean = false)
    private var sentenceQueue: LinkedBlockingQueue<SentenceJob>? = null

    // Page boundary tracking (playback-based)
    private val pageBoundaryIndices = mutableListOf<Int>()
    private var boundariesSignaled = 0

    var availableVoices: List<ServerVoice> = emptyList()
        private set

    fun updateServerUrl(url: String) { serverUrl = url }

    suspend fun fetchVoices(): List<ServerVoice> = withContext(Dispatchers.IO) {
        try {
            val conn = URL("$serverUrl/api/voices").openConnection() as HttpURLConnection
            conn.connectTimeout = 3000; conn.readTimeout = 3000
            val json = conn.inputStream.bufferedReader().readText()
            conn.disconnect()
            val arr = JSONArray(json)
            val voices = (0 until arr.length()).map { i ->
                val obj = arr.getJSONObject(i)
                ServerVoice(obj.getString("id"), obj.getString("name"), obj.getString("lang"))
            }
            availableVoices = voices
            voices
        } catch (e: Exception) { emptyList() }
    }

    /**
     * Start a new continuous playback session.
     * Call appendPage() to add more pages, finishSession() when book ends.
     */
    fun speak(text: String, voiceId: String, speed: Float, skipSentences: Int = 0) {
        val myGen = generationId.incrementAndGet()
        speakJob?.cancel()
        paused = false

        val splitSentences = TtsEngine.splitIntoSentences(text)
        if (splitSentences.isEmpty()) { state = TtsState.FINISHED; return }

        sentences = splitSentences.toMutableList()
        totalSentences = sentences.size
        currentSentence = skipSentences
        pageBoundaryIndices.clear()
        boundariesSignaled = 0

        val queue = LinkedBlockingQueue<SentenceJob>(20)
        sentenceQueue = queue

        // Queue first page's sentences
        for ((i, s) in splitSentences.withIndex()) {
            queue.put(SentenceJob(s, i, isPageBoundary = false))
        }

        state = TtsState.PLAYING

        fun isCurrent() = generationId.get() == myGen

        speakJob = scope.launch {
            // Producer: pulls from queue, fetches audio from server
            data class AudioChunk(val idx: Int, val samples: ShortArray, val sampleRate: Int)
            val audioQueue = LinkedBlockingQueue<AudioChunk>(5)
            var producerDone = false

            val producer = launch(Dispatchers.IO) {
                var successCount = 0
                var consecutiveFailures = 0

                while (isCurrent()) {
                    val job = try {
                        queue.poll(500, TimeUnit.MILLISECONDS)
                    } catch (_: InterruptedException) { null }

                    if (job == null) {
                        // Check if session was ended
                        if (!isCurrent()) break
                        continue
                    }
                    if (job.isEnd) break
                    if (job.globalIdx < skipSentences) continue

                    val wavData = requestTts(job.sentence, voiceId, speed)
                    if (!isCurrent()) break

                    if (wavData == null) {
                        consecutiveFailures++
                        if (consecutiveFailures >= 3) {
                            state = TtsState.ERROR
                            break
                        }
                        continue
                    }
                    consecutiveFailures = 0
                    successCount++

                    val parsed = wavToSamples(wavData) ?: continue
                    audioQueue.put(AudioChunk(job.globalIdx, parsed.first, parsed.second))
                }

                if (successCount == 0 && isCurrent() && state != TtsState.ERROR) {
                    state = TtsState.ERROR
                }
                producerDone = true
            }

            // Consumer: plays audio continuously
            val consumer = launch(Dispatchers.IO) {
                var currentRate = 0
                var track: AudioTrack? = null

                fun ensureTrack(rate: Int): AudioTrack {
                    if (rate != currentRate || track == null) {
                        track?.stop(); track?.release()
                        currentRate = rate
                        val bufSize = AudioTrack.getMinBufferSize(
                            rate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT
                        ).coerceAtLeast(8192)
                        val t = AudioTrack.Builder()
                            .setAudioAttributes(AudioAttributes.Builder()
                                .setUsage(AudioAttributes.USAGE_MEDIA)
                                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
                            .setAudioFormat(AudioFormat.Builder()
                                .setSampleRate(rate)
                                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                                .setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
                            .setBufferSizeInBytes(bufSize)
                            .setTransferMode(AudioTrack.MODE_STREAM).build()
                        t.play()
                        track = t; return t
                    }
                    return track!!
                }

                while (isCurrent()) {
                    val chunk = audioQueue.poll(200, TimeUnit.MILLISECONDS)
                    if (chunk == null) {
                        if (producerDone && audioQueue.isEmpty()) break
                        continue
                    }

                    currentSentence = chunk.idx
                    val t = ensureTrack(chunk.sampleRate)
                    var written = 0
                    while (written < chunk.samples.size && isCurrent()) {
                        while (paused && isCurrent()) { Thread.sleep(50) }
                        if (!isCurrent()) break
                        val result = t.write(chunk.samples, written, minOf(4096, chunk.samples.size - written))
                        if (result > 0) written += result
                    }
                }

                track?.stop(); track?.release()
            }

            producer.join()
            consumer.join()

            if (isCurrent() && state != TtsState.ERROR) {
                state = TtsState.FINISHED
            }
        }
    }

    /** Append another page to the running session. Audio continues without interruption. */
    fun appendPage(text: String) {
        val newSentences = TtsEngine.splitIntoSentences(text)
        if (newSentences.isEmpty()) return

        val offset = sentences.size
        sentences.addAll(newSentences)
        totalSentences = sentences.size

        // Record page boundary
        pageBoundaryIndices.add(offset)

        val queue = sentenceQueue ?: return
        for ((i, s) in newSentences.withIndex()) {
            queue.put(SentenceJob(s, offset + i, isPageBoundary = i == 0))
        }
    }

    /** Signal that no more pages are coming. Worker will finish after current queue. */
    fun finishSession() {
        sentenceQueue?.put(SentenceJob("", -1, false, isEnd = true))
    }

    /** Check if playback has crossed a page boundary (based on currentSentence, not generation). */
    fun checkPageBoundary(): Boolean {
        while (boundariesSignaled < pageBoundaryIndices.size) {
            val boundary = pageBoundaryIndices[boundariesSignaled]
            if (currentSentence >= boundary) {
                boundariesSignaled++
                return true
            }
            break
        }
        return false
    }

    private fun wavToSamples(wavData: ByteArray): Pair<ShortArray, Int>? {
        if (wavData.size < 44) return null
        val sampleRate = (wavData[24].toInt() and 0xFF) or
                ((wavData[25].toInt() and 0xFF) shl 8) or
                ((wavData[26].toInt() and 0xFF) shl 16) or
                ((wavData[27].toInt() and 0xFF) shl 24)
        val numSamples = (wavData.size - 44) / 2
        val samples = ShortArray(numSamples) { i ->
            val offset = 44 + i * 2
            (wavData[offset].toInt() and 0xFF or ((wavData[offset + 1].toInt()) shl 8)).toShort()
        }
        return Pair(samples, sampleRate)
    }

    private fun requestTts(text: String, voice: String, speed: Float): ByteArray? {
        return try {
            val conn = URL("$serverUrl/api/tts").openConnection() as HttpURLConnection
            conn.requestMethod = "POST"
            conn.setRequestProperty("Content-Type", "application/json")
            conn.connectTimeout = 10000; conn.readTimeout = 30000; conn.doOutput = true
            conn.outputStream.write(JSONObject().apply {
                put("text", text); put("voice", voice); put("speed", speed.toDouble())
            }.toString().toByteArray())
            if (conn.responseCode != 200) { conn.disconnect(); return null }
            val out = ByteArrayOutputStream()
            conn.inputStream.use { it.copyTo(out) }
            conn.disconnect()
            out.toByteArray()
        } catch (e: Exception) { null }
    }

    fun pause() { paused = true; state = TtsState.PAUSED }
    fun resume(onStateChange: () -> Unit) { paused = false; state = TtsState.PLAYING }
    fun stop() { generationId.incrementAndGet(); paused = false; speakJob?.cancel(); sentenceQueue = null; state = TtsState.IDLE }
    fun getCurrentSentenceText(): String? = sentences.getOrNull(currentSentence)
    fun release() { stop(); scope.cancel() }
}
