package dev.fetchx.downloads.settings

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONObject

/**
 * Persists [FolderCategoriesConfig] in SharedPreferences as a JSON blob.
 */
class FolderCategoriesRepository(context: Context) {

    private val prefs: SharedPreferences =
        context.getSharedPreferences("fetchx_folder_categories", Context.MODE_PRIVATE)

    fun load(): FolderCategoriesConfig {
        val json = prefs.getString(KEY_CONFIG, null) ?: return FolderCategoriesConfig()
        return try {
            val obj = JSONObject(json)
            FolderCategoriesConfig(
                enabled = obj.optBoolean("enabled", false),
                compressed = parseRule(obj.optJSONObject("compressed"), "Compressed", "zip,rar,7z,gz,gzip,tar,tgz,bz2,xz,z,cab,arj,lzh,lha,ace,sit,sitx,jar"),
                documents = parseRule(obj.optJSONObject("documents"), "Documents", "pdf,doc,docx,xls,xlsx,ppt,pptx,txt,rtf,odt,ods,odp,csv,epub,chm"),
                music = parseRule(obj.optJSONObject("music"), "Music", "mp3,wav,wma,ogg,flac,aac,m4a,mid,midi,aif,aiff,ra,ape,opus"),
                programs = parseRule(obj.optJSONObject("programs"), "Programs", "exe,msi,dmg,pkg,deb,rpm,apk,appimage,bin,iso,bat"),
                video = parseRule(obj.optJSONObject("video"), "Video", "mp4,mkv,avi,mov,wmv,flv,webm,mpg,mpeg,m4v,3gp,ts,vob,asf,rm,ogv"),
                images = parseRule(obj.optJSONObject("images"), "Images", "jpg,jpeg,png,gif,bmp,webp,svg,ico,tiff,tif,heic,heif,raw,cr2,nef"),
                custom = obj.optString("custom", ""),
            )
        } catch (_: Exception) {
            FolderCategoriesConfig()
        }
    }

    fun save(config: FolderCategoriesConfig) {
        val obj = JSONObject()
        obj.put("enabled", config.enabled)
        obj.put("compressed", ruleToJson(config.compressed))
        obj.put("documents", ruleToJson(config.documents))
        obj.put("music", ruleToJson(config.music))
        obj.put("programs", ruleToJson(config.programs))
        obj.put("video", ruleToJson(config.video))
        obj.put("images", ruleToJson(config.images))
        obj.put("custom", config.custom)
        prefs.edit().putString(KEY_CONFIG, obj.toString()).apply()
    }

    private fun parseRule(obj: JSONObject?, defaultFolder: String, defaultExts: String): CategoryRule {
        if (obj == null) return CategoryRule(defaultFolder, defaultExts)
        return CategoryRule(
            folder = obj.optString("folder", defaultFolder),
            extensions = obj.optString("extensions", defaultExts),
        )
    }

    private fun ruleToJson(rule: CategoryRule): JSONObject {
        val obj = JSONObject()
        obj.put("folder", rule.folder)
        obj.put("extensions", rule.extensions)
        return obj
    }

    private companion object {
        const val KEY_CONFIG = "config"
    }
}
