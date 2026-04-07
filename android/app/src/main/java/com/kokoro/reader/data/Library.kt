package com.kokoro.reader.data

import android.net.Uri
import android.content.Context
import com.google.gson.Gson
import java.io.File

class Library(private val dataDir: File) {

    private val gson = Gson()
    private val libraryFile get() = File(dataDir, "library.json")
    private val booksDir get() = File(dataDir, "books")

    var books: MutableList<BookEntry> = mutableListOf()
        private set

    init {
        booksDir.mkdirs()
        load()
    }

    private fun load() {
        books = if (libraryFile.exists()) {
            try {
                val data = gson.fromJson(libraryFile.readText(), LibraryData::class.java)
                data.books.filter { File(booksDir, it.filename).exists() }.toMutableList()
            } catch (e: Exception) {
                mutableListOf()
            }
        } else {
            mutableListOf()
        }
    }

    fun save() {
        dataDir.mkdirs()
        libraryFile.writeText(gson.toJson(LibraryData(books)))
    }

    fun import(context: Context, uri: Uri, filename: String): String? {
        // Check if already imported
        books.find { it.filename == filename }?.let { return it.id }

        // Copy file
        val dest = File(booksDir, filename)
        booksDir.mkdirs()
        context.contentResolver.openInputStream(uri)?.use { input ->
            dest.outputStream().use { output -> input.copyTo(output) }
        } ?: return null

        val id = filename.hashCode().toString(16)
        val title = filename.removeSuffix(".pdf").removeSuffix(".PDF")

        val entry = BookEntry(
            id = id,
            title = title,
            filename = filename,
            total_pages = 0, // will be set when opened
            last_page = 0,
            last_sentence = 0,
            selected_voice = 0,
            last_accessed = System.currentTimeMillis() / 1000
        )
        books.add(0, entry)
        save()
        return id
    }

    fun delete(id: String) {
        books.find { it.id == id }?.let { book ->
            File(booksDir, book.filename).delete()
            books.removeAll { it.id == id }
            save()
        }
    }

    fun updateProgress(id: String, page: Int, sentence: Int, voice: Int) {
        books.find { it.id == id }?.let { book ->
            val idx = books.indexOf(book)
            books[idx] = book.copy(
                last_page = page,
                last_sentence = sentence,
                selected_voice = voice,
                last_accessed = System.currentTimeMillis() / 1000
            )
            save()
        }
    }

    fun getBookPath(book: BookEntry): File = File(booksDir, book.filename)

    fun reload() = load()
}
