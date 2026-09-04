package com.deepscan.mobile

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

/** Remote client to a desktop DeepScan engine on the local network — see mobile/README.md. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { DeepScanApp() }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DeepScanApp() {
    var engineUrl by remember { mutableStateOf("http://192.168.1.1:51424") }
    var query by remember { mutableStateOf("") }
    var results by remember { mutableStateOf<List<SearchResult>>(emptyList()) }
    var status by remember { mutableStateOf("not connected") }
    val scope = rememberCoroutineScope()

    MaterialTheme {
        Surface(modifier = Modifier.fillMaxSize()) {
            Column(modifier = Modifier.padding(20.dp)) {
                Text("DeepScan", style = MaterialTheme.typography.headlineMedium)
                Text(status, style = MaterialTheme.typography.bodySmall)

                Spacer(Modifier.height(16.dp))
                OutlinedTextField(
                    value = engineUrl,
                    onValueChange = { engineUrl = it },
                    label = { Text("Desktop engine address") },
                    modifier = Modifier.fillMaxWidth(),
                )

                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    label = { Text("Search your desktop's files") },
                    modifier = Modifier.fillMaxWidth(),
                )

                Spacer(Modifier.height(8.dp))
                Button(onClick = {
                    scope.launch {
                        DeepScanClient(engineUrl).search(query).fold(
                            onSuccess = { results = it; status = "${it.size} results" },
                            onFailure = { status = "search failed: ${it.message}" },
                        )
                    }
                }) { Text("Search") }

                Spacer(Modifier.height(16.dp))
                LazyColumn {
                    items(results) { result ->
                        Column(modifier = Modifier.padding(vertical = 10.dp)) {
                            Text(result.category.uppercase(), style = MaterialTheme.typography.labelSmall)
                            Text(result.path, style = MaterialTheme.typography.bodyMedium)
                            TextButton(onClick = {
                                scope.launch { DeepScanClient(engineUrl).revealOnDesktop(result.path) }
                            }) { Text("reveal on desktop") }
                        }
                    }
                }
            }
        }
    }
}
