package dev.fetchx.app.aria2

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun Aria2RemoteScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val savedConfig = remember(context) { Aria2RemotePreferences.load(context) }
    var rpcUrl by remember { mutableStateOf(savedConfig.url) }
    var secret by remember { mutableStateOf(savedConfig.secret) }
    var status by remember { mutableStateOf<String?>(null) }
    var testing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            OutlinedButton(onClick = onBack) { Text("Back") }
            Button(
                enabled = !testing,
                onClick = {
                    testing = true
                    status = null
                    val config = Aria2RemoteConfig(rpcUrl, secret)
                    scope.launch {
                        try {
                            val version = withContext(Dispatchers.IO) {
                                Aria2RemotePreferences.save(context, config)
                                callAria2(config, "aria2.getVersion")
                                    .getJSONObject("result")
                                    .optString("version", "unknown")
                            }
                            val stats = withContext(Dispatchers.IO) {
                                callAria2(config, "aria2.getGlobalStat")
                                    .getJSONObject("result")
                            }
                            val active = stats.optString("numActive")
                            val waiting = stats.optString("numWaiting")
                            val stopped = stats.optString("numStopped")
                            status = "Connected · aria2 $version · active $active · waiting $waiting · stopped $stopped"
                        } catch (error: Exception) {
                            status = "Connection failed: ${error.message ?: "unknown error"}"
                        } finally {
                            testing = false
                        }
                    }
                },
            ) { Text(if (testing) "Testing..." else "Test connection") }
        }

        Text("Remote aria2", style = MaterialTheme.typography.headlineSmall)
        OutlinedTextField(
            modifier = Modifier.fillMaxWidth(),
            value = rpcUrl,
            onValueChange = { rpcUrl = it },
            label = { Text("JSON-RPC URL") },
            placeholder = { Text("http://server:6800/jsonrpc") },
            singleLine = true,
        )
        OutlinedTextField(
            modifier = Modifier.fillMaxWidth(),
            value = secret,
            onValueChange = { secret = it },
            label = { Text("RPC secret") },
            visualTransformation = PasswordVisualTransformation(),
            singleLine = true,
        )
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text("Connection status", style = MaterialTheme.typography.titleMedium)
                Text(status ?: "Enter the server URL and test the connection.")
            }
        }
    }
}
