package com.kokoro.reader

import android.os.Bundle
import androidx.activity.ComponentActivity
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
import java.io.File

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        setContent {
            KokoroTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    val prefs = getSharedPreferences("kokoro", MODE_PRIVATE)
                    var dataDir by remember {
                        mutableStateOf(prefs.getString("data_dir", null))
                    }

                    val navController = rememberNavController()

                    // If no data dir configured, show settings first
                    val startDest = if (dataDir == null) "settings" else "library"

                    NavHost(navController, startDestination = startDest) {
                        composable("settings") {
                            SettingsScreen(
                                currentDir = dataDir ?: "",
                                onSave = { dir ->
                                    prefs.edit().putString("data_dir", dir).apply()
                                    dataDir = dir
                                    navController.navigate("library") {
                                        popUpTo("settings") { inclusive = true }
                                    }
                                }
                            )
                        }

                        composable("library") {
                            val dir = dataDir ?: return@composable
                            val library = remember(dir) { Library(File(dir)) }
                            LibraryScreen(
                                library = library,
                                onOpenBook = { bookId ->
                                    navController.navigate("reader/$bookId")
                                },
                                onSettings = {
                                    navController.navigate("settings")
                                },
                                context = this@MainActivity
                            )
                        }

                        composable("reader/{bookId}") { backStackEntry ->
                            val bookId = backStackEntry.arguments?.getString("bookId") ?: return@composable
                            val dir = dataDir ?: return@composable
                            val library = remember(dir) { Library(File(dir)) }
                            ReaderScreen(
                                library = library,
                                bookId = bookId,
                                onBack = { navController.popBackStack() }
                            )
                        }
                    }
                }
            }
        }
    }
}
