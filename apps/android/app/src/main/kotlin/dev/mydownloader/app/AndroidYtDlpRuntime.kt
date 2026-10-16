package dev.fetchx.app

import android.content.Context
import android.os.Build
import android.util.Log
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import dev.fetchx.bridge.uniffi.YtDlpRuntime
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
        val runtimeArguments = if (requiresFfmpeg(arguments)) {
            val withFfmpeg = arguments.toMutableList()
            val urlDelimiter = withFfmpeg.indexOf("--")
            check(urlDelimiter >= 0) { "yt-dlp arguments are missing the URL delimiter" }
            withFfmpeg.add(urlDelimiter, "--ffmpeg-location")
            withFfmpeg.add(urlDelimiter + 1, ensureFfmpegExecutable().absolutePath)
            withFfmpeg
        } else {
            arguments
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
        Log.e("FetchXYtDlp", "Rust yt-dlp failure code=$diagnosticCode")
    }

    @Synchronized
    private fun ensureFfmpegExecutable(): File {
        val abi = Build.SUPPORTED_ABIS.firstOrNull { it in SUPPORTED_FFMPEG_ABIS }
            ?: error("No bundled FFmpeg executable for this device ABI")
        val bundled = File(context.applicationInfo.nativeLibraryDir, FFMPEG_LIBRARY_NAME)
        check(bundled.isFile) { "Bundled FFmpeg executable is missing for $abi" }

        val directory = File(context.filesDir, "runtime/ffmpeg")
        check(directory.isDirectory || directory.mkdirs()) {
            "Could not create the FFmpeg runtime directory"
        }
        val executable = File(directory, "ffmpeg")
        if (!executable.isFile || executable.length() != bundled.length()) {
            val temporary = File(directory, "ffmpeg.tmp")
            bundled.copyTo(temporary, overwrite = true)
            temporary.copyTo(executable, overwrite = true)
            check(executable.setExecutable(true, true)) {
                "Could not make the FFmpeg executable runnable"
            }
            check(temporary.delete()) { "Could not clean up the FFmpeg staging file" }
        } else if (!executable.canExecute()) {
            check(executable.setExecutable(true, true)) {
                "Could not make the FFmpeg executable runnable"
            }
        }
        return executable
    }

    companion object {
        private const val FFMPEG_LIBRARY_NAME = "libfetchx_ffmpeg.so"
        private val SUPPORTED_FFMPEG_ABIS = setOf("arm64-v8a", "x86_64")

        fun start(context: Context): AndroidYtDlpRuntime {
            if (!Python.isStarted()) {
                Python.start(AndroidPlatform(context.applicationContext))
            }
            check(Python.isStarted()) { "Embedded Python failed to start" }
            return AndroidYtDlpRuntime().apply {
                this.context = context.applicationContext
            }
        }

        private fun requiresFfmpeg(arguments: List<String>): Boolean =
            arguments.windowed(2).any { (option, value) ->
                option == "--format" && "+bestaudio" in value
            }
        }
}
