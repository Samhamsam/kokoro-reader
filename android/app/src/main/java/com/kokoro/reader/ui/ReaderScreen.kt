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
import com.kokoro.reader.tts.TtsEngine
import com.kokoro.reader.tts.TtsState
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

    // TTS
    val ttsEngine = remember { TtsEngine(context) }
    var ttsState by remember { mutableStateOf(TtsState.IDLE) }
    var currentSentenceIdx by remember { mutableIntStateOf(0) }
    var readingActive by remember { mutableStateOf(false) }
    val onStateChange: () -> Unit = {
        ttsState = ttsEngine.state
        currentSentenceIdx = ttsEngine.currentSentence
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
        val result = withContext(Dispatchers.IO) {
            renderPage(pdfFile, currentPage)
        }
        bitmap = result?.first
        pageText = result?.second ?: ""
        loading = false

        // Update total pages
        if (totalPages == 0) {
            totalPages = withContext(Dispatchers.IO) { getPageCount(pdfFile) }
        }

        // Save progress
        library.updateProgress(bookId, currentPage, 0, book.selected_voice)

        // Auto-read if reading active
        if (readingActive && pageText.isNotBlank()) {
            ttsEngine.speak(pageText, onStateChange)
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            ttsEngine.stop()
            library.updateProgress(bookId, currentPage, ttsEngine.currentSentence, book.selected_voice)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        TextButton(onClick = {
                            ttsEngine.stop()
                            library.updateProgress(bookId, currentPage, ttsEngine.currentSentence, book.selected_voice)
                            onBack()
                        }) {
                            Text("< Library", color = TextPrimary)
                        }
                        Spacer(Modifier.width(8.dp))

                        // Page nav
                        IconButton(
                            onClick = {
                                ttsEngine.stop()
                                readingActive = false
                                if (currentPage > 0) currentPage--
                            },
                            enabled = currentPage > 0
                        ) { Text("<", color = TextPrimary, fontSize = 18.sp) }

                        Text("${currentPage + 1}/$totalPages", color = TextDim, fontSize = 14.sp)

                        IconButton(
                            onClick = {
                                ttsEngine.stop()
                                readingActive = false
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
            // TTS Controls
            Surface(color = Surface, tonalElevation = 4.dp) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    when (ttsState) {
                        TtsState.IDLE, TtsState.FINISHED, TtsState.ERROR -> {
                            Button(
                                onClick = {
                                    readingActive = true
                                    ttsEngine.setSpeed(speed)
                                    ttsEngine.speak(pageText, onStateChange)
                                },
                                enabled = pageText.isNotBlank(),
                                colors = ButtonDefaults.buttonColors(containerColor = Green)
                            ) { Text("Play") }
                        }
                        TtsState.PLAYING, TtsState.GENERATING -> {
                            Button(
                                onClick = { ttsEngine.pause(); onStateChange() },
                                colors = ButtonDefaults.buttonColors(containerColor = Amber)
                            ) { Text("Pause") }
                            Button(
                                onClick = {
                                    ttsEngine.stop()
                                    readingActive = false
                                    onStateChange()
                                },
                                colors = ButtonDefaults.buttonColors(containerColor = Red)
                            ) { Text("Stop") }
                        }
                        TtsState.PAUSED -> {
                            Button(
                                onClick = { ttsEngine.resume(onStateChange) },
                                colors = ButtonDefaults.buttonColors(containerColor = Green)
                            ) { Text("Resume") }
                            Button(
                                onClick = {
                                    ttsEngine.stop()
                                    readingActive = false
                                    onStateChange()
                                },
                                colors = ButtonDefaults.buttonColors(containerColor = Red)
                            ) { Text("Stop") }
                        }
                    }

                    Spacer(Modifier.weight(1f))

                    // Speed
                    Text("${String.format("%.1f", speed)}x", color = TextDim, fontSize = 12.sp)
                    Slider(
                        value = speed,
                        onValueChange = {
                            speed = it
                            ttsEngine.setSpeed(it)
                        },
                        valueRange = 0.5f..2.0f,
                        steps = 5,
                        modifier = Modifier.width(100.dp)
                    )
                }
            }
        }
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
            contentAlignment = Alignment.TopCenter
        ) {
            if (loading) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(color = Accent)
                }
            } else {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                ) {
                    // PDF Page
                    bitmap?.let { bmp ->
                        Image(
                            bitmap = bmp.asImageBitmap(),
                            contentDescription = "Page ${currentPage + 1}",
                            modifier = Modifier.fillMaxWidth()
                        )
                    }

                    // Current sentence indicator
                    if (ttsState == TtsState.PLAYING || ttsState == TtsState.PAUSED) {
                        ttsEngine.getCurrentSentenceText()?.let { sentence ->
                            Card(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(8.dp),
                                colors = CardDefaults.cardColors(
                                    containerColor = Accent.copy(alpha = 0.15f)
                                )
                            ) {
                                Text(
                                    sentence,
                                    modifier = Modifier.padding(12.dp),
                                    color = TextPrimary,
                                    fontSize = 14.sp
                                )
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
        val bitmap = Bitmap.createBitmap(
            page.width * scale, page.height * scale, Bitmap.Config.ARGB_8888
        )
        bitmap.eraseColor(android.graphics.Color.WHITE)
        page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)

        // Extract text using PDFBox
        val text = try {
            val doc = PDDocument.load(file)
            val stripper = PDFTextStripper()
            stripper.startPage = idx + 1
            stripper.endPage = idx + 1
            val t = stripper.getText(doc)
            doc.close()
            t.trim()
        } catch (e: Exception) { "" }

        page.close()
        renderer.close()
        fd.close()
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
