package com.kokoro.reader.ui

import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import com.tom_roush.pdfbox.android.PDFBoxResourceLoader
import com.tom_roush.pdfbox.pdmodel.PDDocument
import com.tom_roush.pdfbox.text.PDFTextStripper
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.kokoro.reader.data.Library
import com.kokoro.reader.tts.SherpaOnnxTts
import com.kokoro.reader.tts.TtsState
import com.kokoro.reader.tts.VoiceType
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    library: Library,
    bookId: String,
    onBack: () -> Unit
) {
    val book = library.books.find { it.id == bookId } ?: run {
        onBack()
        return
    }
    val pdfFile = library.getBookPath(book)
    val context = LocalContext.current

    var currentPage by remember { mutableIntStateOf(book.last_page) }
    var totalPages by remember { mutableIntStateOf(book.total_pages) }
    var bitmap by remember { mutableStateOf<Bitmap?>(null) }
    var pageText by remember { mutableStateOf("") }
    var loading by remember { mutableStateOf(true) }
    var speed by remember { mutableFloatStateOf(1.0f) }
    var selectedVoice by remember {
        val savedId = book.selected_voice_id
        val voice = VoiceType.entries.find { it.name == savedId } ?: VoiceType.KOKORO_HEART
        mutableStateOf(voice)
    }
    var voiceMenuExpanded by remember { mutableStateOf(false) }

    // TTS engines
    val modelsDir = remember { File(context.filesDir, "tts-models") }
    val sherpaEngine = remember { SherpaOnnxTts(modelsDir) }
    val systemEngine = remember { TtsEngine(context) }
    var ttsState by remember { mutableStateOf(TtsState.IDLE) }
    var currentSentenceIdx by remember { mutableIntStateOf(0) }
    var downloadPct by remember { mutableIntStateOf(0) }
    var readingActive by remember { mutableStateOf(false) }

    // Callbacks for both engines
    val onSherpaStateChange: () -> Unit = {
        ttsState = sherpaEngine.state
        currentSentenceIdx = sherpaEngine.currentSentence
    }
    val onSystemStateChange: () -> Unit = {
        ttsState = systemEngine.state
        currentSentenceIdx = systemEngine.currentSentence
    }

    // Helper: start TTS with the right engine
    fun startTts(text: String) {
        if (selectedVoice.isSystem) {
            val locale = if (selectedVoice == VoiceType.SYSTEM_DE) java.util.Locale.GERMAN else java.util.Locale.US
            systemEngine.setLanguage(locale)
            systemEngine.setSpeed(speed)
            systemEngine.speak(text, onSystemStateChange)
        } else {
            sherpaEngine.speak(text, selectedVoice, speed, onStateChange = onSherpaStateChange)
        }
    }

    fun stopTts() {
        systemEngine.stop()
        sherpaEngine.stop()
        ttsState = TtsState.IDLE
    }

    fun pauseTts() {
        if (selectedVoice.isSystem) { systemEngine.pause() } else { sherpaEngine.pause() }
        ttsState = TtsState.PAUSED
    }

    fun resumeTts() {
        if (selectedVoice.isSystem) {
            systemEngine.resume(onSystemStateChange)
        } else {
            sherpaEngine.resume(onSherpaStateChange)
        }
        ttsState = TtsState.PLAYING
    }

    fun getCurrentSentence(): String? {
        return if (selectedVoice.isSystem) systemEngine.getCurrentSentenceText()
        else sherpaEngine.getCurrentSentenceText()
    }

    // Poll download progress while generating
    LaunchedEffect(ttsState) {
        if (ttsState == TtsState.GENERATING) {
            while (sherpaEngine.state == TtsState.GENERATING) {
                downloadPct = (sherpaEngine.downloadProgress * 100).toInt()
                kotlinx.coroutines.delay(200)
            }
            ttsState = sherpaEngine.state
        }
    }

    // Auto-advance when page finished
    LaunchedEffect(ttsState) {
        if (ttsState == TtsState.FINISHED && readingActive) {
            if (currentPage + 1 < totalPages) {
                currentPage++
            } else {
                readingActive = false
            }
        }
    }

    // Render page
    LaunchedEffect(currentPage) {
        loading = true
        val result = withContext(Dispatchers.IO) { renderPage(pdfFile, currentPage) }
        bitmap = result?.first
        pageText = result?.second ?: ""
        loading = false

        if (totalPages == 0) {
            totalPages = withContext(Dispatchers.IO) { getPageCount(pdfFile) }
        }

        library.updateProgress(bookId, currentPage, 0, selectedVoice.name)

        if (readingActive && pageText.isNotBlank()) {
            ttsEngine.speak(pageText, selectedVoice, speed, onStateChange = onStateChange)
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            ttsEngine.stop()
            library.updateProgress(bookId, currentPage, ttsEngine.currentSentence, selectedVoice.name)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        TextButton(onClick = {
                            ttsEngine.stop()
                            library.updateProgress(bookId, currentPage, ttsEngine.currentSentence, selectedVoice.name)
                            onBack()
                        }) { Text("< Library", color = TextPrimary) }

                        Spacer(Modifier.width(8.dp))

                        IconButton(
                            onClick = {
                                ttsEngine.stop(); readingActive = false
                                if (currentPage > 0) currentPage--
                            },
                            enabled = currentPage > 0
                        ) { Text("<", color = TextPrimary, fontSize = 18.sp) }

                        Text("${currentPage + 1}/$totalPages", color = TextDim, fontSize = 14.sp)

                        IconButton(
                            onClick = {
                                ttsEngine.stop(); readingActive = false
                                if (currentPage + 1 < totalPages) currentPage++
                            },
                            enabled = currentPage + 1 < totalPages
                        ) { Text(">", color = TextPrimary, fontSize = 18.sp) }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Surface)
            )
        },
        bottomBar = {
            Surface(color = Surface, tonalElevation = 4.dp) {
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    // TTS Controls
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        when (ttsState) {
                            TtsState.IDLE, TtsState.FINISHED, TtsState.ERROR -> {
                                Button(
                                    onClick = {
                                        readingActive = true
                                        ttsEngine.speak(pageText, selectedVoice, speed, onStateChange = onStateChange)
                                    },
                                    enabled = pageText.isNotBlank(),
                                    colors = ButtonDefaults.buttonColors(containerColor = Green)
                                ) { Text("Play") }
                            }
                            TtsState.GENERATING -> {
                                if (downloadPct in 1..99) {
                                    CircularProgressIndicator(
                                        progress = { downloadPct / 100f },
                                        modifier = Modifier.size(24.dp),
                                        color = Amber,
                                        strokeWidth = 2.dp
                                    )
                                    Text(
                                        "Downloading $downloadPct%",
                                        color = Amber,
                                        fontSize = 12.sp
                                    )
                                } else {
                                    CircularProgressIndicator(
                                        modifier = Modifier.size(24.dp),
                                        color = Amber,
                                        strokeWidth = 2.dp
                                    )
                                    Text("Loading model...", color = Amber, fontSize = 12.sp)
                                }
                            }
                            TtsState.PLAYING -> {
                                Button(
                                    onClick = { ttsEngine.pause(); onStateChange() },
                                    colors = ButtonDefaults.buttonColors(containerColor = Amber)
                                ) { Text("Pause") }
                                Button(
                                    onClick = { ttsEngine.stop(); readingActive = false; onStateChange() },
                                    colors = ButtonDefaults.buttonColors(containerColor = Red)
                                ) { Text("Stop") }
                            }
                            TtsState.PAUSED -> {
                                Button(
                                    onClick = { ttsEngine.resume(onStateChange) },
                                    colors = ButtonDefaults.buttonColors(containerColor = Green)
                                ) { Text("Resume") }
                                Button(
                                    onClick = { ttsEngine.stop(); readingActive = false; onStateChange() },
                                    colors = ButtonDefaults.buttonColors(containerColor = Red)
                                ) { Text("Stop") }
                            }
                        }

                        Spacer(Modifier.weight(1f))

                        // Voice selector
                        Box {
                            TextButton(onClick = { voiceMenuExpanded = true }) {
                                Text(selectedVoice.label, color = TextPrimary, fontSize = 12.sp)
                            }
                            DropdownMenu(
                                expanded = voiceMenuExpanded,
                                onDismissRequest = { voiceMenuExpanded = false }
                            ) {
                                VoiceType.entries.forEach { voice ->
                                    val downloaded = voice.isDownloaded(modelsDir)
                                    DropdownMenuItem(
                                        text = {
                                            Row(
                                                modifier = Modifier.fillMaxWidth(),
                                                horizontalArrangement = Arrangement.SpaceBetween
                                            ) {
                                                Text(voice.label)
                                                if (!downloaded) {
                                                    Text(
                                                        "${voice.sizeMB} MB",
                                                        color = TextDim,
                                                        fontSize = 11.sp
                                                    )
                                                }
                                            }
                                        },
                                        onClick = {
                                            selectedVoice = voice
                                            voiceMenuExpanded = false
                                        },
                                        leadingIcon = if (downloaded) {
                                            { Text("✓", color = Green, fontSize = 14.sp) }
                                        } else null
                                    )
                                }
                            }
                        }
                    }

                    // Speed slider
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Speed: ${String.format("%.1f", speed)}x", color = TextDim, fontSize = 12.sp)
                        Slider(
                            value = speed,
                            onValueChange = { speed = it },
                            valueRange = 0.5f..2.0f,
                            steps = 5,
                            modifier = Modifier.weight(1f)
                        )
                    }
                }
            }
        }
    ) { padding ->
        Box(
            modifier = Modifier.fillMaxSize().padding(padding),
            contentAlignment = Alignment.TopCenter
        ) {
            if (loading) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(color = Accent)
                }
            } else {
                Column(modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                    bitmap?.let { bmp ->
                        Image(
                            bitmap = bmp.asImageBitmap(),
                            contentDescription = "Page ${currentPage + 1}",
                            modifier = Modifier.fillMaxWidth()
                        )
                    }

                    // Current sentence
                    if (ttsState == TtsState.PLAYING || ttsState == TtsState.PAUSED) {
                        ttsEngine.getCurrentSentenceText()?.let { sentence ->
                            Card(
                                modifier = Modifier.fillMaxWidth().padding(8.dp),
                                colors = CardDefaults.cardColors(containerColor = Accent.copy(alpha = 0.15f))
                            ) {
                                Text(sentence, modifier = Modifier.padding(12.dp), color = TextPrimary, fontSize = 14.sp)
                            }
                        }
                    }
                }
            }
        }
    }
}

private fun renderPage(file: File, pageIndex: Int): Pair<Bitmap, String>? {
    return try {
        val fd = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        val renderer = PdfRenderer(fd)
        val idx = pageIndex.coerceIn(0, renderer.pageCount - 1)
        val page = renderer.openPage(idx)

        val scale = 2
        val bitmap = Bitmap.createBitmap(page.width * scale, page.height * scale, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(android.graphics.Color.WHITE)
        page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
        page.close()
        renderer.close()
        fd.close()

        val text = try {
            val doc = PDDocument.load(file)
            val stripper = PDFTextStripper()
            stripper.startPage = idx + 1
            stripper.endPage = idx + 1
            val t = stripper.getText(doc)
            doc.close()
            t.trim()
        } catch (e: Exception) { "" }

        Pair(bitmap, text)
    } catch (e: Exception) {
        e.printStackTrace()
        null
    }
}

private fun getPageCount(file: File): Int {
    return try {
        val fd = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        val renderer = PdfRenderer(fd)
        val count = renderer.pageCount
        renderer.close()
        fd.close()
        count
    } catch (e: Exception) { 0 }
}
