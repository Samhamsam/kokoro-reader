package com.kokoro.reader.tts

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import com.k2fsa.sherpa.onnx.*
import kotlinx.coroutines.*
import java.io.File
import java.net.URL

enum class ModelType { SYSTEM, KOKORO, PIPER }

enum class VoiceType(
    val label: String,
    val modelUrl: String,
    val modelDir: String,
    val sizeMB: Int,
    val type: ModelType,
    val modelFile: String = "",
    val speakerId: Int = 0,
) {
    // System TTS (Android built-in, fast, works everywhere)
    SYSTEM_EN("EN System", "", "", 0, ModelType.SYSTEM),
    SYSTEM_DE("DE System", "", "", 0, ModelType.SYSTEM),

    // Kokoro English voices (slow on weak devices!)
    KOKORO_HEART("EN Heart (F) [305MB]", "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-en-v0_19.tar.bz2",
        "kokoro-en-v0_19", 305, ModelType.KOKORO, "model.onnx", 0),
    KOKORO_ADAM("EN Adam (M) [305MB]", "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-en-v0_19.tar.bz2",
        "kokoro-en-v0_19", 305, ModelType.KOKORO, "model.onnx", 5),

    // Piper (faster on mobile than Kokoro)
    PIPER_THORSTEN("DE Thorsten (M) [111MB]", "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-de_DE-thorsten-high.tar.bz2",
        "vits-piper-de_DE-thorsten-high", 111, ModelType.PIPER, "de_DE-thorsten-high.onnx"),

    PIPER_SIWIS("FR Siwis (F) [74MB]", "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-fr_FR-siwis-medium.tar.bz2",
        "vits-piper-fr_FR-siwis-medium", 74, ModelType.PIPER, "fr_FR-siwis-medium.onnx");

    val isSystem get() = type == ModelType.SYSTEM
    fun isDownloaded(modelsDir: File): Boolean = isSystem || File(modelsDir, modelDir).exists()
}

class SherpaOnnxTts(private val modelsBaseDir: File) {
    private var tts: OfflineTts? = null
    private var currentVoice: VoiceType? = null
    private var audioTrack: AudioTrack? = null

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

    val isModelReady: Boolean get() = tts != null
    var downloadProgress: Float = 0f
        private set

    suspend fun loadModel(voice: VoiceType): Boolean {
        // Same model dir = same model, just different speaker
        if (currentVoice?.modelDir == voice.modelDir && tts != null) {
            currentVoice = voice
            return true
        }

        tts?.release()
        tts = null

        val modelDir = File(modelsBaseDir, voice.modelDir)
        if (!modelDir.exists()) {
            state = TtsState.GENERATING
            downloadProgress = 0f
            val success = downloadAndExtract(voice.modelUrl, voice.sizeMB, modelsBaseDir)
            if (!success) {
                state = TtsState.ERROR
                return false
            }
        }

        val config = when (voice.type) {
            ModelType.KOKORO -> OfflineTtsConfig(
                model = OfflineTtsModelConfig(
                    kokoro = OfflineTtsKokoroModelConfig(
                        model = File(modelDir, voice.modelFile).absolutePath,
                        voices = File(modelDir, "voices.bin").absolutePath,
                        tokens = File(modelDir, "tokens.txt").absolutePath,
                        dataDir = File(modelDir, "espeak-ng-data").absolutePath,
                    ),
                    numThreads = 2,
                ),
            )
            ModelType.PIPER -> OfflineTtsConfig(
                model = OfflineTtsModelConfig(
                    vits = OfflineTtsVitsModelConfig(
                        model = File(modelDir, voice.modelFile).absolutePath,
                        tokens = File(modelDir, "tokens.txt").absolutePath,
                        dataDir = File(modelDir, "espeak-ng-data").absolutePath,
                    ),
                    numThreads = 2,
                ),
            )
        }

        try {
            tts = OfflineTts(config = config)
            currentVoice = voice
            state = TtsState.IDLE
            return true
        } catch (e: Exception) {
            e.printStackTrace()
            state = TtsState.ERROR
            return false
        }
    }

    fun speak(text: String, voice: VoiceType, speed: Float, skipSentences: Int = 0, onStateChange: () -> Unit) {
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
        state = TtsState.GENERATING

        val speakerId = voice.speakerId

        speakJob = scope.launch {
            // Ensure model is loaded
            if (!loadModel(voice)) {
                state = TtsState.ERROR
                withContext(Dispatchers.Main) { onStateChange() }
                return@launch
            }

            val localTts = tts ?: return@launch
            val sampleRate = localTts.sampleRate()

            for ((i, sentence) in sentences.withIndex()) {
                if (i < skipSentences) continue
                if (stopped) break

                while (paused && !stopped) {
                    delay(100)
                }
                if (stopped) break

                currentSentence = i
                if (i == 0 || i == skipSentences) {
                    state = TtsState.PLAYING
                }
                withContext(Dispatchers.Main) { onStateChange() }

                // Generate audio
                val audio = localTts.generate(sentence, sid = speakerId, speed = speed)

                if (stopped) break

                // Play audio
                playAudio(audio.samples, sampleRate)
            }

            if (!stopped) {
                state = TtsState.FINISHED
                withContext(Dispatchers.Main) { onStateChange() }
            }
        }
        onStateChange()
    }

    private fun playAudio(samples: FloatArray, sampleRate: Int) {
        val shortSamples = ShortArray(samples.size) {
            (samples[it] * 32767f).toInt().coerceIn(-32768, 32767).toShort()
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

        // Wait for playback to finish
        val durationMs = (samples.size.toLong() * 1000) / sampleRate
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
        tts?.release()
        scope.cancel()
    }

    private suspend fun downloadAndExtract(url: String, sizeMB: Int, destDir: File): Boolean {
        return withContext(Dispatchers.IO) {
            try {
                val tarFile = File(destDir, "model-download.tar.bz2")
                destDir.mkdirs()

                // Download with progress tracking
                val connection = URL(url).openConnection()
                val totalBytes = sizeMB.toLong() * 1024 * 1024
                var downloaded = 0L
                connection.getInputStream().use { input ->
                    tarFile.outputStream().use { output ->
                        val buffer = ByteArray(8192)
                        var read: Int
                        while (input.read(buffer).also { read = it } != -1) {
                            output.write(buffer, 0, read)
                            downloaded += read
                            downloadProgress = if (totalBytes > 0) {
                                (downloaded.toFloat() / totalBytes).coerceIn(0f, 1f)
                            } else 0f
                        }
                    }
                }

                downloadProgress = 1f

                // Extract
                val process = ProcessBuilder("tar", "xf", tarFile.absolutePath, "-C", destDir.absolutePath)
                    .start()
                process.waitFor()
                tarFile.delete()
                process.exitValue() == 0
            } catch (e: Exception) {
                e.printStackTrace()
                false
            }
        }
    }
}
