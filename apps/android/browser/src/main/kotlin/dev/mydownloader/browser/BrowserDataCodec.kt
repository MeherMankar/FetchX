package dev.fetchx.browser

internal object BrowserDataCodec {
    fun encodeTabs(tabs: List<TabState>): String = encodeRecords("v2", tabs.map {
        listOf(it.id, it.url, it.title.orEmpty(), it.spaceId, it.desktopSiteEnabled.toString())
    })

    fun decodeTabs(value: String): List<TabState> {
        if (!value.startsWith("v1:") && !value.startsWith("v2:")) {
            return value.split("\n").mapNotNull { line ->
                val fields = line.split("\t\t")
                if (fields.size < 4) null else TabState(
                    id = fields[0],
                    url = fields[1],
                    title = fields[2].ifBlank { null },
                    spaceId = fields[3],
                )
            }.distinctBy { it.id }
        }
        val version = if (value.startsWith("v2:")) "v2" else "v1"
        return decodeRecords(value, version).mapNotNull { fields ->
            if (version == "v1" && fields.size != 4) return@mapNotNull null
            if (version == "v2" && fields.size != 5) return@mapNotNull null
            TabState(
                id = fields[0],
                url = fields[1],
                title = fields[2].ifBlank { null },
                spaceId = fields[3],
                desktopSiteEnabled = fields.getOrNull(4)?.toBooleanStrictOrNull() ?: false,
            )
        }.distinctBy { it.id }
    }

    fun encodeBookmarks(bookmarks: List<Bookmark>): String = encodeRecords("v1", bookmarks.map {
        listOf(it.id, it.url, it.title, it.createdAtMs.toString(), it.updatedAtMs.toString())
    })

    fun decodeBookmarks(value: String): List<Bookmark> {
        if (!value.startsWith("v1:")) {
            return value.split("\n").mapNotNull { line ->
                val fields = line.split("\t\t")
                if (fields.size < 5) return@mapNotNull null
                val createdAt = fields[3].toLongOrNull() ?: return@mapNotNull null
                val updatedAt = fields[4].toLongOrNull() ?: return@mapNotNull null
                Bookmark(fields[0], fields[1], fields[2], createdAt, updatedAt)
            }
        }
        return decodeRecords(value, "v1").mapNotNull { fields ->
            if (fields.size != 5) return@mapNotNull null
            val createdAt = fields[3].toLongOrNull() ?: return@mapNotNull null
            val updatedAt = fields[4].toLongOrNull() ?: return@mapNotNull null
            Bookmark(fields[0], fields[1], fields[2], createdAt, updatedAt)
        }
    }

    fun encodeHistory(entries: List<HistoryEntry>): String = encodeRecords("v1", entries.map {
        listOf(it.id, it.url, it.title.orEmpty(), it.timestampMs.toString())
    })

    fun decodeHistory(value: String): List<HistoryEntry> = decodeRecords(value, "v1").mapNotNull { fields ->
        if (fields.size != 4) return@mapNotNull null
        val timestamp = fields[3].toLongOrNull() ?: return@mapNotNull null
        HistoryEntry(fields[0], fields[1], fields[2].ifBlank { null }, timestamp)
    }

    private fun encodeRecords(version: String, records: List<List<String>>): String =
        "$version:" + records.joinToString(",") { record ->
            record.joinToString(".") { field ->
                java.util.Base64.getUrlEncoder().withoutPadding()
                    .encodeToString(field.toByteArray(Charsets.UTF_8))
            }
        }

    private fun decodeRecords(value: String, version: String): List<List<String>> {
        val prefix = "$version:"
        if (!value.startsWith(prefix)) return emptyList()
        return value.removePrefix(prefix).split(',').filter { it.isNotEmpty() }.mapNotNull { record ->
            runCatching {
                record.split('.').map { field ->
                    String(java.util.Base64.getUrlDecoder().decode(field), Charsets.UTF_8)
                }
            }.getOrNull()
        }
    }
}
