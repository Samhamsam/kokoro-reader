package com.kokoro.reader.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(currentServerUrl: String, onSave: (serverUrl: String) -> Unit) {
    var serverUrl by remember { mutableStateOf(currentServerUrl.ifEmpty { "http://192.168.1.100:8787" }) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Surface)
            )
        }
    ) { padding ->
        Column(modifier = Modifier.fillMaxSize().padding(padding).padding(20.dp)) {
            Text("Server URL", style = MaterialTheme.typography.titleMedium, color = TextPrimary)
            Spacer(Modifier.height(4.dp))
            Text(
                "URL of the Kokoro Server (books, TTS, progress).",
                style = MaterialTheme.typography.bodySmall, color = TextDim
            )
            Spacer(Modifier.height(12.dp))
            OutlinedTextField(
                value = serverUrl,
                onValueChange = { serverUrl = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                label = { Text("URL") },
                placeholder = { Text("http://192.168.1.100:8787") }
            )
            Spacer(Modifier.height(32.dp))
            Button(
                onClick = { onSave(serverUrl) },
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(containerColor = Accent)
            ) {
                Text("Save & Continue", fontSize = 16.sp)
            }
        }
    }
}
