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

data class ServerVoice(val id: String, val name: String, val lang: String)

class ServerTtsEngine(private var serverUrl: String) {
    var state: TtsState = TtsState.IDLE
        private set
    var currentSentence: Int = 0
        private set
    var totalSentences: Int = 0
        private set

    private var sentences: List<String> = emptyList()
    @Volatile private var stopped = false
    @Volatile private var paused = false
    private var speakJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    var availableVoices: List<ServerVoice> = emptyList()
        private set

    fun updateServerUrl(url: String) { serverUrl = url }

    suspend fun fetchVoices(): List<ServerVoice> = withContext(Dispatchers.IO) {
        try {
            val conn = URL("$serverUrl/voices").openConnection() as HttpURLConnection
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
        speakJob?.cancel()
        stopped = true // signal old job to stop
        Thread.sleep(50) // let old job notice

        val splitSentences = TtsEngine.splitIntoSentences(text)
        if (splitSentences.isEmpty()) { state = TtsState.FINISHED; return }

        sentences = splitSentences
        totalSentences = sentences.size
        currentSentence = skipSentences
        stopped = false
        paused = false
        state = TtsState.PLAYING

        // Queue of raw PCM samples (short arrays) with sentence index
        // null = end of stream
        val pcmQueue = LinkedBlockingQueue<Pair<Int, ShortArray>?>(5)

        speakJob = scope.launch {
            // PRODUCER: fetch audio from server, decode WAV, put PCM in queue
            val producer = launch(Dispatchers.IO) {
                for ((i, sentence) in sentences.withIndex()) {
                    if (i < skipSentences) continue
                    if (stopped) break

                    val wavData = requestTts(sentence, voiceId, speed)
                    if (wavData == null || stopped) break

                    val pcm = wavToShortArray(wavData)
                    if (pcm != null && !stopped) {
                        pcmQueue.put(Pair(i, pcm)) // blocks if queue is full
                    }
                }
                pcmQueue.put(null) // end sentinel
            }

            // CONSUMER: single AudioTrack, continuously fed with PCM
            launch(Dispatchers.IO) {
                val sampleRate = 24000 // Kokoro default; Piper is 22050 but resampling is close enough
                val bufSize = AudioTrack.getMinBufferSize(
                    sampleRate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT
                ).coerceAtLeast(8192)

                val track = AudioTrack.Builder()
                    .setAudioAttributes(AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
                    .setAudioFormat(AudioFormat.Builder()
                        .setSampleRate(sampleRate)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
                    .setBufferSizeInBytes(bufSize)
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    .build()

                track.play()

                while (!stopped) {
                    val item = pcmQueue.poll(100, java.util.concurrent.TimeUnit.MILLISECONDS)
                    if (item == null && pcmQueue.isEmpty()) {
                        // Check if producer is done
                        if (!producer.isActive) break
                        continue
                    }
                    if (item == null) break // end sentinel

                    val (sentenceIdx, samples) = item
                    currentSentence = sentenceIdx

                    // Write PCM to AudioTrack — this blocks naturally until played
                    var written = 0
                    while (written < samples.size && !stopped) {
                        while (paused && !stopped) { Thread.sleep(50) }
                        if (stopped) break

                        val result = track.write(samples, written,
                            minOf(4096, samples.size - written))
                        if (result > 0) written += result
                    }
                }

                track.stop()
                track.release()
            }

            producer.join()

            if (!stopped) {
                // Wait for consumer to finish playing
                while (!pcmQueue.isEmpty()) { delay(100) }
                delay(200) // let last audio drain
                state = TtsState.FINISHED
            }
        }
    }

    private fun wavToShortArray(wavData: ByteArray): ShortArray? {
        if (wavData.size < 44) return null
        val numSamples = (wavData.size - 44) / 2
        return ShortArray(numSamples) { i ->
            val offset = 44 + i * 2
            (wavData[offset].toInt() and 0xFF or ((wavData[offset + 1].toInt()) shl 8)).toShort()
        }
    }

    private fun requestTts(text: String, voice: String, speed: Float): ByteArray? {
        return try {
            val conn = URL("$serverUrl/tts").openConnection() as HttpURLConnection
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
    fun stop() { stopped = true; paused = false; speakJob?.cancel(); state = TtsState.IDLE }
    fun getCurrentSentenceText(): String? = sentences.getOrNull(currentSentence)
    fun release() { stop(); scope.cancel() }
}
