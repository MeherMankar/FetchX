package dev.fetchx.app

import android.content.Context
import android.util.Log
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import dev.fetchx.bridge.uniffi.YtDlpRuntime

class AndroidYtDlpRuntime : YtDlpRuntime {
    private val module = Python.getInstance().getModule("fetchx_ytdlp")

    override fun validateVersion(): String =
        module.callAttr("validate_version").toString()

    override fun run(
        arguments: List<String>,
        progressPath: String,
        cancelPath: String,
    ): String {
        val result = module.callAttr(
            "run",
            arguments.toTypedArray(),
            progressPath,
            cancelPath,
        ).toString()
        if (result.isNotEmpty()) {
            Log.e("FetchXYtDlp", result)
        }
        return result
    }

    companion object {
        fun start(context: Context): AndroidYtDlpRuntime {
            if (!Python.isStarted()) {
                Python.start(AndroidPlatform(context.applicationContext))
            }
            check(Python.isStarted()) { "Embedded Python failed to start" }
            return AndroidYtDlpRuntime()
        }
    }
}
