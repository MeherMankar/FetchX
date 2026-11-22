package dev.fetchx.downloads.settings

import java.io.File

object FolderCategoriser {
    /**
     * Returns the destination path for [file] given [config], or null if no
     * rule matches, categorisation is disabled, or the file is already in
     * the target folder.
     */
    fun categorise(file: File, config: FolderCategoriesConfig): File? {
        if (!config.enabled) return null
        val ext = file.extension.lowercase().ifEmpty { return null }
        val rules = buildRules(config)
        val folder = rules[ext] ?: return null
        val parent = file.parentFile ?: return null
        val destDir = if (File(folder).isAbsolute) File(folder) else File(parent, folder)
        val alreadyInFolder = if (File(folder).isAbsolute) {
            parent.canonicalPath == destDir.canonicalPath
        } else {
            parent.name.equals(folder, ignoreCase = true)
        }
        if (alreadyInFolder) return null
        return File(destDir, file.name)
    }

    private fun buildRules(config: FolderCategoriesConfig): Map<String, String> {
        val rules = mutableMapOf<String, String>()
        listOf(
            config.compressed,
            config.documents,
            config.music,
            config.programs,
            config.video,
            config.images,
        ).forEach { rule ->
            rule.extensions.split(",").map { it.trim().lowercase() }.filter { it.isNotEmpty() }
                .forEach { ext -> rules[ext] = rule.folder }
        }
        config.custom.split(";").forEach { entry ->
            val eqIdx = entry.indexOf('=')
            if (eqIdx < 1) return@forEach
            val folder = entry.substring(0, eqIdx).trim()
            val exts = entry.substring(eqIdx + 1)
            if (folder.isEmpty()) return@forEach
            exts.split(",").map { it.trim().lowercase() }.filter { it.isNotEmpty() }
                .forEach { ext -> rules[ext] = folder }
        }
        return rules
    }
}
