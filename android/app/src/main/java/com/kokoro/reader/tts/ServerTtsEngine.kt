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
import java.util.concurrent.atomic.AtomicLong

data class ServerVoice(val id: String, val name: String, val lang: String)

class ServerTtsEngine(private var serverUrl: String) {
    var state: TtsState = TtsState.IDLE
        private set
    var currentSentence: Int = 0
        private set
    var totalSentences: Int = 0
        private set

    private var sentences: List<String> = emptyList()
    private val generationId = AtomicLong(0) // incremented on each speak()
    @Volatile private var paused = false
    private var speakJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

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

    fun speak(text: String, voiceId: String, speed: Float, skipSentences: Int = 0, onStateChange: () -> Unit) {
        // New generation — invalidates any old running job
        val myGen = generationId.incrementAndGet()
        speakJob?.cancel()
        paused = false
        state = TtsState.PLAYING

        val splitSentences = TtsEngine.splitIntoSentences(text)
        if (splitSentences.isEmpty()) { state = TtsState.FINISHED; return }

        sentences = splitSentences
        totalSentences = sentences.size
        currentSentence = skipSentences

        data class AudioChunk(val idx: Int, val samples: ShortArray, val sampleRate: Int)
        val pcmQueue = LinkedBlockingQueue<AudioChunk?>(5)

        fun isCurrent() = generationId.get() == myGen

        speakJob = scope.launch {
            val producer = launch(Dispatchers.IO) {
                for ((i, sentence) in sentences.withIndex()) {
                    if (i < skipSentences) continue
                    if (!isCurrent()) return@launch

                    val wavData = requestTts(sentence, voiceId, speed)
                    if (wavData == null || !isCurrent()) return@launch

                    val parsed = wavToSamples(wavData) ?: continue
                    pcmQueue.put(AudioChunk(i, parsed.first, parsed.second))
                }
                pcmQueue.put(null)
            }

            launch(Dispatchers.IO) {
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
                    val chunk = pcmQueue.poll(100, java.util.concurrent.TimeUnit.MILLISECONDS)
                    if (chunk == null) {
                        if (pcmQueue.isEmpty() && !producer.isActive) break
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
            if (isCurrent()) {
                while (!pcmQueue.isEmpty()) { delay(100) }
                delay(200)
                state = TtsState.FINISHED
            }
        }
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
    fun stop() { generationId.incrementAndGet(); paused = false; speakJob?.cancel(); state = TtsState.IDLE }
    fun getCurrentSentenceText(): String? = sentences.getOrNull(currentSentence)
    fun release() { stop(); scope.cancel() }
}
