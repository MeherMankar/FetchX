package dev.fetchx.browser

internal object BrowserDataCodec {
    fun encodeTabs(tabs: List<TabState>): String = encodeRecords(tabs.map {
        listOf(it.id, it.url, it.title.orEmpty(), it.spaceId)
    })

    fun decodeTabs(value: String): List<TabState> {
        if (!value.startsWith("v1:")) {
            return value.split("\n").mapNotNull { line ->
                val fields = line.split("\t\t")
                if (fields.size < 4) null else TabState(
                    id = fields[0],
                    url = fields[1],
                    title = fields[2].ifBlank { null },
                    spaceId = fields[3],
                )
            }
        }
        return decodeRecords(value).mapNotNull { fields ->
            if (fields.size != 4) return@mapNotNull null
            TabState(
                id = fields[0],
                url = fields[1],
                title = fields[2].ifBlank { null },
                spaceId = fields[3],
            )
        }
    }

    fun encodeBookmarks(bookmarks: List<Bookmark>): String = encodeRecords(bookmarks.map {
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
        return decodeRecords(value).mapNotNull { fields ->
            if (fields.size != 5) return@mapNotNull null
            val createdAt = fields[3].toLongOrNull() ?: return@mapNotNull null
            val updatedAt = fields[4].toLongOrNull() ?: return@mapNotNull null
            Bookmark(fields[0], fields[1], fields[2], createdAt, updatedAt)
        }
    }

    fun encodeHistory(entries: List<HistoryEntry>): String = encodeRecords(entries.map {
        listOf(it.id, it.url, it.title.orEmpty(), it.timestampMs.toString())
    })

    fun decodeHistory(value: String): List<HistoryEntry> = decodeRecords(value).mapNotNull { fields ->
        if (fields.size != 4) return@mapNotNull null
        val timestamp = fields[3].toLongOrNull() ?: return@mapNotNull null
        HistoryEntry(fields[0], fields[1], fields[2].ifBlank { null }, timestamp)
    }

    private fun encodeRecords(records: List<List<String>>): String =
        "v1:" + records.joinToString(",") { record ->
            record.joinToString(".") { field ->
                java.util.Base64.getUrlEncoder().withoutPadding()
                    .encodeToString(field.toByteArray(Charsets.UTF_8))
            }
        }

    private fun decodeRecords(value: String): List<List<String>> {
        if (!value.startsWith("v1:")) return emptyList()
        return value.removePrefix("v1:").split(',').filter { it.isNotEmpty() }.mapNotNull { record ->
            runCatching {
                record.split('.').map { field ->
                    String(java.util.Base64.getUrlDecoder().decode(field), Charsets.UTF_8)
                }
            }.getOrNull()
        }
    }
}
