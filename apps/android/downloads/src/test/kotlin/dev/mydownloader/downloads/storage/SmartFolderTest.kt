package dev.fetchx.downloads.storage

import org.junit.Assert.assertEquals
import org.junit.Test

class SmartFolderTest {
    @Test
    fun `classifies common media and document files`() {
        assertEquals("Videos", smartFolderName("movie.mkv"))
        assertEquals("Audio", smartFolderName("song.opus"))
        assertEquals("Archives", smartFolderName("bundle.zip"))
        assertEquals("Documents", smartFolderName("manual.pdf"))
        assertEquals("Others", smartFolderName("image.bin"))
    }
}
