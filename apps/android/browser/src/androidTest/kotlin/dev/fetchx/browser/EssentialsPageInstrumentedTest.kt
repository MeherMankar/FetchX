package dev.fetchx.browser

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class EssentialsPageInstrumentedTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun startPageSearchTopSiteAndDownloadsUseExistingActions() {
        var searchCalls = 0
        var downloadsCalls = 0
        var openedUrl: String? = null

        composeRule.setContent {
            MaterialTheme {
                EssentialsPage(
                    recentEntries = listOf(
                        Essential(
                            title = "Example",
                            url = "https://example.com",
                            initial = "E",
                            colorHex = "#4285F4",
                        ),
                    ),
                    ongoingDownloadCount = 2,
                    selectedSearchEngine = SearchEngine.DUCKDUCKGO,
                    profileName = "Default",
                    profileAccentColor = Color(0xFF4F8EF7),
                    onSearch = { searchCalls++ },
                    onNavigateRecent = { openedUrl = it },
                    onOpenBookmarks = {},
                    onOpenHistory = {},
                    onOpenDownloads = { downloadsCalls++ },
                )
            }
        }

        composeRule.onNodeWithContentDescription("Search or type URL").performClick()
        composeRule.onNodeWithContentDescription("Open Example").performClick()
        composeRule.onNodeWithContentDescription("Downloads, 2 ongoing").performClick()
        composeRule.onNodeWithText("Top sites").assertIsDisplayed()

        composeRule.runOnIdle {
            assertEquals(1, searchCalls)
            assertEquals(1, downloadsCalls)
            assertEquals("https://example.com", openedUrl)
        }
    }
}
