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

data class ServerVoice(
    val id: String,
    val name: String,
    val lang: String
)

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
    private var audioTrack: AudioTrack? = null
    private var speakJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    var availableVoices: List<ServerVoice> = emptyList()
        private set

    fun updateServerUrl(url: String) {
        serverUrl = url
    }

    suspend fun fetchVoices(): List<ServerVoice> {
        return withContext(Dispatchers.IO) {
            try {
                val url = URL("$serverUrl/voices")
                val conn = url.openConnection() as HttpURLConnection
                conn.connectTimeout = 3000
                conn.readTimeout = 3000
                val json = conn.inputStream.bufferedReader().readText()
                conn.disconnect()

                val arr = JSONArray(json)
                val voices = mutableListOf<ServerVoice>()
                for (i in 0 until arr.length()) {
                    val obj = arr.getJSONObject(i)
                    voices.add(ServerVoice(
                        id = obj.getString("name").lowercase()
                            .replace(" ", "_").replace("(", "").replace(")", ""),
                        name = obj.getString("name"),
                        lang = obj.getString("lang")
                    ))
                }
                availableVoices = voices
                voices
            } catch (e: Exception) {
                e.printStackTrace()
                emptyList()
            }
        }
    }

    fun speak(text: String, voiceId: String, speed: Float, skipSentences: Int = 0, onStateChange: () -> Unit) {
        stop()
        val splitSentences = TtsEngine.splitIntoSentences(text)
        if (splitSentences.isEmpty()) {
            state = TtsState.FINISHED
            onStateChange()
            return
        }

        sentences = splitSentences
        totalSentences = sentences.size
        currentSentence = 0
        stopped = false
        paused = false
        state = TtsState.PLAYING

        speakJob = scope.launch {
            for ((i, sentence) in sentences.withIndex()) {
                if (i < skipSentences) continue
                if (stopped) break

                while (paused && !stopped) { delay(100) }
                if (stopped) break

                currentSentence = i
                withContext(Dispatchers.Main) { onStateChange() }

                // Request audio from server
                val wavData = requestTts(sentence, voiceId, speed)
                if (wavData == null || stopped) {
                    if (!stopped) {
                        state = TtsState.ERROR
                        withContext(Dispatchers.Main) { onStateChange() }
                    }
                    break
                }

                if (stopped) break

                // Play WAV
                playWav(wavData)
            }

            if (!stopped) {
                state = TtsState.FINISHED
                withContext(Dispatchers.Main) { onStateChange() }
            }
        }
        onStateChange()
    }

    private fun requestTts(text: String, voice: String, speed: Float): ByteArray? {
        return try {
            val url = URL("$serverUrl/tts")
            val conn = url.openConnection() as HttpURLConnection
            conn.requestMethod = "POST"
            conn.setRequestProperty("Content-Type", "application/json")
            conn.connectTimeout = 10000
            conn.readTimeout = 30000
            conn.doOutput = true

            val json = JSONObject().apply {
                put("text", text)
                put("voice", voice)
                put("speed", speed.toDouble())
            }
            conn.outputStream.write(json.toString().toByteArray())

            if (conn.responseCode != 200) {
                conn.disconnect()
                return null
            }

            val out = ByteArrayOutputStream()
            conn.inputStream.use { it.copyTo(out) }
            conn.disconnect()
            out.toByteArray()
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    private fun playWav(wavData: ByteArray) {
        if (wavData.size < 44) return

        // Parse WAV header
        val sampleRate = wavData[24].toInt() and 0xFF or
                ((wavData[25].toInt() and 0xFF) shl 8) or
                ((wavData[26].toInt() and 0xFF) shl 16) or
                ((wavData[27].toInt() and 0xFF) shl 24)
        val dataSize = wavData.size - 44
        val numSamples = dataSize / 2

        val shortSamples = ShortArray(numSamples)
        for (i in 0 until numSamples) {
            val offset = 44 + i * 2
            shortSamples[i] = (wavData[offset].toInt() and 0xFF or
                    ((wavData[offset + 1].toInt()) shl 8)).toShort()
        }

        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build()
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(sampleRate)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .build()
            )
            .setBufferSizeInBytes(shortSamples.size * 2)
            .setTransferMode(AudioTrack.MODE_STATIC)
            .build()

        audioTrack = track
        track.write(shortSamples, 0, shortSamples.size)
        track.play()

        val durationMs = (numSamples.toLong() * 1000) / sampleRate
        Thread.sleep(durationMs)
        track.release()
        audioTrack = null
    }

    fun pause() {
        paused = true
        audioTrack?.pause()
        state = TtsState.PAUSED
    }

    fun resume(onStateChange: () -> Unit) {
        paused = false
        audioTrack?.play()
        state = TtsState.PLAYING
        onStateChange()
    }

    fun stop() {
        stopped = true
        paused = false
        speakJob?.cancel()
        audioTrack?.stop()
        audioTrack?.release()
        audioTrack = null
        state = TtsState.IDLE
    }

    fun getCurrentSentenceText(): String? = sentences.getOrNull(currentSentence)

    fun release() {
        stop()
        scope.cancel()
    }
}
