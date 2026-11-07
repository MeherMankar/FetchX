package dev.fetchx.downloads.ui

import kotlin.test.assertEquals
import kotlin.test.assertTrue
import org.junit.Test

class BulkLinkParserTest {
    @Test
    fun extractsUniqueHttpLinksFromMixedText() {
        val links = BulkLinkParser.extract(
            """
            https://example.com/a.zip
            mirrored copy: https://example.com/a.zip
            https://example.com/b%20two.iso
            ignore ftp://example.com/file
            """.trimIndent(),
        )

        assertEquals(
            listOf(
                "https://example.com/a.zip",
                "https://example.com/b%20two.iso",
            ),
            links,
        )
    }

    @Test
    fun suggestedFileNameSanitizesUnsafeCharacters() {
        val name = BulkLinkParser.suggestedFileName("https://example.com/path/my:file?.mp4?download=1")
        assertTrue(name.all { it != ':' && it != '?' && it != '/' })
    }
}
