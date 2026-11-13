package dev.fetchx.app

import android.content.Context
import android.os.Build
import android.util.Log
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import dev.fetchx.bridge.uniffi.YtDlpRuntime
import dev.fetchx.app.media.MediaDownloadPreferences
import java.io.File

class AndroidYtDlpRuntime : YtDlpRuntime {
    private lateinit var context: Context
    private val module = Python.getInstance().getModule("fetchx_ytdlp")

    override fun validateVersion(): String =
        module.callAttr("validate_version").toString()

    override fun run(
        arguments: List<String>,
        progressPath: String,
        cancelPath: String,
    ): String {
        val mediaArguments = MediaDownloadPreferences.applyArguments(context, arguments)
        val runtimeArguments = if (requiresNativeMediaProcessor(mediaArguments)) {
            val withNativeProcessor = mediaArguments.toMutableList()
            val urlDelimiter = withNativeProcessor.indexOf("--")
            check(urlDelimiter >= 0) { "yt-dlp arguments are missing the URL delimiter" }
            withNativeProcessor.add(urlDelimiter, "--ffmpeg-location")
            withNativeProcessor.add(
                urlDelimiter + 1,
                nativeMediaProcessorForYtDlp().absolutePath,
            )
            withNativeProcessor
        } else {
            mediaArguments
        }
        val result = module.callAttr(
            "run",
            runtimeArguments.toTypedArray(),
            progressPath,
            cancelPath,
        ).toString()
        if (result.isNotEmpty()) {
            Log.e("FetchXYtDlp", result)
        }
        return result
    }

    override fun discoverFormats(url: String): String {
        val result = module.callAttr("discover_formats", url).toString()
        if (result.startsWith("ERROR:")) {
            Log.e("FetchXYtDlp", "yt-dlp format discovery failed")
        }
        return result
    }

    override fun reportFailure(diagnosticCode: String) {
        Log.e("FetchXYtDlp", "Rust yt-dlp diagnostic=$diagnosticCode")
    }

    private fun nativeMediaProcessorForYtDlp(): File {
        val abi = Build.SUPPORTED_ABIS.firstOrNull { it in SUPPORTED_MEDIA_ABIS }
            ?: error("Native media processing is not supported for this device ABI")
        val executable = File(
            context.applicationInfo.nativeLibraryDir,
            NATIVE_MEDIA_PROCESSOR_LIBRARY_NAME,
        )
        check(executable.isFile) {
            "Native media processor is missing for $abi"
        }
        check(executable.canExecute()) { "Native media processor is not runnable for $abi" }
        return executable
    }

    companion object {
        private const val NATIVE_MEDIA_PROCESSOR_LIBRARY_NAME = "libfetchx_ffmpeg_native.so"
        private val SUPPORTED_MEDIA_ABIS = setOf("arm64-v8a", "x86_64")

        fun start(context: Context): AndroidYtDlpRuntime {
            if (!Python.isStarted()) {
                Python.start(AndroidPlatform(context.applicationContext))
            }
            check(Python.isStarted()) { "Embedded Python failed to start" }
            return AndroidYtDlpRuntime().apply {
                this.context = context.applicationContext
            }
        }

        internal fun requiresNativeMediaProcessor(arguments: List<String>): Boolean =
            "--extract-audio" in arguments ||
                arguments.windowed(2).any { (option, value) ->
                    option == "--format" && '+' in value
                }
    }
}
