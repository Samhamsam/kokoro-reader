package com.kokoro.reader.data

import android.content.Context
import android.net.Uri
import com.google.gson.Gson
import com.google.gson.reflect.TypeToken
import java.io.*
import java.net.HttpURLConnection
import java.net.URL

class Library(private val serverUrl: String, private val cacheDir: File) {

    private val gson = Gson()
    var books: MutableList<BookEntry> = mutableListOf()
        private set

    init {
        File(cacheDir, "books").mkdirs()
        // Don't call refresh() here — it does network I/O.
        // Callers must call refresh() from a background thread/coroutine.
    }

    data class ProgressSyncResult(
        val book: BookEntry? = null,
        val conflicted: Boolean = false,
        val failed: Boolean = false
    )

    fun refresh() {
        try {
            val conn = URL("$serverUrl/api/books").openConnection() as HttpURLConnection
            conn.connectTimeout = 5000; conn.readTimeout = 5000
            val json = conn.inputStream.bufferedReader().readText()
            conn.disconnect()
            val type = object : TypeToken<List<BookEntry>>() {}.type
            books = gson.fromJson<List<BookEntry>>(json, type).toMutableList()
            pruneCache()
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    fun fetchBook(id: String): BookEntry? {
        return try {
            val conn = URL("$serverUrl/api/books/$id").openConnection() as HttpURLConnection
            conn.connectTimeout = 5000; conn.readTimeout = 5000
            if (conn.responseCode !in 200..299) {
                conn.disconnect()
                return null
            }
            val json = conn.inputStream.bufferedReader().readText()
            conn.disconnect()
            val book = gson.fromJson(json, BookEntry::class.java)
            upsertBook(book)
            book
        } catch (e: Exception) {
            e.printStackTrace()
            null
        }
    }

    fun import(context: Context, uri: Uri, filename: String): String? {
        try {
            val boundary = "---KokoroUpload${System.currentTimeMillis()}"
            val conn = URL("$serverUrl/api/books").openConnection() as HttpURLConnection
            conn.requestMethod = "POST"
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "multipart/form-data; boundary=$boundary")
            conn.connectTimeout = 30000; conn.readTimeout = 60000

            val outputStream = conn.outputStream
            val writer = BufferedWriter(OutputStreamWriter(outputStream))

            // File part
            writer.write("--$boundary\r\n")
            writer.write("Content-Disposition: form-data; name=\"file\"; filename=\"$filename\"\r\n")
            writer.write("Content-Type: application/pdf\r\n\r\n")
            writer.flush()

            context.contentResolver.openInputStream(uri)?.use { it.copyTo(outputStream) }
            outputStream.flush()

            writer.write("\r\n--$boundary--\r\n")
            writer.flush()
            writer.close()

            if (conn.responseCode != 201) {
                conn.disconnect()
                return null
            }

            val respJson = conn.inputStream.bufferedReader().readText()
            conn.disconnect()
            val book = gson.fromJson(respJson, BookEntry::class.java)
            books.add(0, book)
            return book.id
        } catch (e: Exception) {
            e.printStackTrace()
            return null
        }
    }

    fun delete(id: String) {
        val ok = try {
            val conn = URL("$serverUrl/api/books/$id").openConnection() as HttpURLConnection
            conn.requestMethod = "DELETE"
            conn.connectTimeout = 5000
            val code = conn.responseCode
            conn.disconnect()
            code in 200..299
        } catch (_: Exception) { false }

        if (ok) {
            books.removeAll { it.id == id }
            File(cacheDir, "books/$id.pdf").delete()
        }
    }

    fun updateProgress(id: String, page: Int, sentence: Int, voiceId: String, speed: Float = 1.0f): ProgressSyncResult {
        return try {
            val baseVersion = books.find { it.id == id }?.version ?: 0L
            val conn = URL("$serverUrl/api/books/$id/progress").openConnection() as HttpURLConnection
            conn.requestMethod = "PUT"
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            conn.connectTimeout = 5000; conn.readTimeout = 5000
            conn.outputStream.write("""{"last_page":$page,"last_sentence":$sentence,"selected_voice_id":"$voiceId","speed":$speed,"base_version":$baseVersion}""".toByteArray())
            val code = conn.responseCode
            if (code in 200..299 || code == 409) {
                val json = (if (code == 409) conn.errorStream else conn.inputStream)
                    ?.bufferedReader()
                    ?.readText()
                if (!json.isNullOrBlank()) {
                    val book = gson.fromJson(json, BookEntry::class.java)
                    upsertBook(book)
                    conn.disconnect()
                    return ProgressSyncResult(book = book, conflicted = code == 409)
                }
            }
            conn.disconnect()
            ProgressSyncResult(failed = code !in 200..299 && code != 409)
        } catch (e: Exception) {
            e.printStackTrace()
            ProgressSyncResult(failed = true)
        }
    }

    fun getBookFile(id: String): File {
        val cached = File(cacheDir, "books/$id.pdf")
        if (cached.exists()) return cached

        // Download from server
        try {
            val conn = URL("$serverUrl/api/books/$id/file").openConnection() as HttpURLConnection
            conn.connectTimeout = 10000; conn.readTimeout = 60000
            conn.inputStream.use { input ->
                cached.outputStream().use { output -> input.copyTo(output) }
            }
            conn.disconnect()
        } catch (e: Exception) {
            e.printStackTrace()
        }
        return cached
    }

    private fun pruneCache() {
        val validIds = books.map { it.id }.toSet()
        File(cacheDir, "books").listFiles()?.forEach { file ->
            if (file.name.endsWith(".pdf")) {
                val id = file.name.removeSuffix(".pdf")
                if (id !in validIds) file.delete()
            }
        }
    }

    private fun upsertBook(book: BookEntry) {
        val idx = books.indexOfFirst { it.id == book.id }
        if (idx >= 0) {
            books[idx] = book
        } else {
            books.add(book)
        }
    }
}
