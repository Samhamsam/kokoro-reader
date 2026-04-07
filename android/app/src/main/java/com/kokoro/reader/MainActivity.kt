package com.kokoro.reader

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import androidx.activity.ComponentActivity
import com.tom_roush.pdfbox.android.PDFBoxResourceLoader
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.kokoro.reader.data.Library
import com.kokoro.reader.ui.LibraryScreen
import com.kokoro.reader.ui.ReaderScreen
import com.kokoro.reader.ui.SettingsScreen
import com.kokoro.reader.ui.KokoroTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        PDFBoxResourceLoader.init(applicationContext)

        setContent {
            KokoroTheme {
                Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    val prefs = getSharedPreferences("kokoro", MODE_PRIVATE)
                    var serverUrl by remember {
                        mutableStateOf(prefs.getString("server_url", null))
                    }

                    val navController = rememberNavController()
                    val startDest = if (serverUrl == null) "settings" else "library"

                    NavHost(navController, startDestination = startDest) {
                        composable("settings") {
                            SettingsScreen(
                                currentServerUrl = serverUrl ?: "",
                                onSave = { url ->
                                    prefs.edit().putString("server_url", url).apply()
                                    serverUrl = url
                                    navController.navigate("library") {
                                        popUpTo("settings") { inclusive = true }
                                    }
                                }
                            )
                        }

                        composable("library") {
                            val url = serverUrl ?: return@composable
                            val library = remember(url) { Library(url, cacheDir) }
                            LibraryScreen(
                                library = library,
                                onOpenBook = { bookId ->
                                    navController.navigate("reader/$bookId")
                                },
                                onSettings = { navController.navigate("settings") },
                                context = this@MainActivity
                            )
                        }

                        composable("reader/{bookId}") { backStackEntry ->
                            val bookId = backStackEntry.arguments?.getString("bookId") ?: return@composable
                            val url = serverUrl ?: return@composable
                            val library = remember(url) { Library(url, cacheDir) }
                            ReaderScreen(
                                library = library,
                                bookId = bookId,
                                serverUrl = url,
                                onBack = { navController.popBackStack() }
                            )
                        }
                    }
                }
            }
        }
    }
}
