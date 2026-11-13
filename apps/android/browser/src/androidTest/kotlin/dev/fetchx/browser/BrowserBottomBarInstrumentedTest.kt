package dev.fetchx.browser

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class BrowserBottomBarInstrumentedTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun bottomBarKeepsFivePrimaryBrowserActions() {
        composeRule.setContent {
            MaterialTheme {
                BrowserBottomBar(
                    canGoBack = false,
                    tabCount = 1,
                    ongoingDownloadCount = 0,
                    onBack = {},
                    onHome = {},
                    onDownloads = {},
                    onTabs = {},
                    onMenu = {},
                )
            }
        }

        composeRule.onNodeWithContentDescription("Back").assertExists()
        composeRule.onNodeWithContentDescription("Home").assertExists()
        composeRule.onNodeWithContentDescription("Tabs, 1 tab").assertExists()
        composeRule.onNodeWithContentDescription("Downloads").assertExists()
        composeRule.onNodeWithContentDescription("Menu").assertExists()
        composeRule.onNodeWithContentDescription("Forward").assertDoesNotExist()
    }

    @Test
    fun tabsControlAnnouncesCurrentTabCount() {
        val tabCount = mutableStateOf(1)

        composeRule.setContent {
            MaterialTheme {
                BrowserBottomBar(
                    canGoBack = false,
                    tabCount = tabCount.value,
                    ongoingDownloadCount = 0,
                    onBack = {},
                    onHome = {},
                    onDownloads = {},
                    onTabs = {},
                    onMenu = {},
                )
            }
        }

        composeRule.onNodeWithContentDescription("Tabs, 1 tab").assertExists()
        composeRule.runOnIdle { tabCount.value = 3 }
        composeRule.onNodeWithContentDescription("Tabs, 3 tabs").assertExists()
    }
}
