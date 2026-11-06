package dev.fetchx.browser

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class BrowserBottomBarInstrumentedTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun forwardActionIsDisabledWithoutHistoryAndInvokedWhenEnabled() {
        val canGoForward = mutableStateOf(false)
        var forwardCalls = 0

        composeRule.setContent {
            MaterialTheme {
                BrowserBottomBar(
                    canGoBack = false,
                    canGoForward = canGoForward.value,
                    tabCount = 1,
                    ongoingDownloadCount = 0,
                    onBack = {},
                    onForward = { forwardCalls++ },
                    onHome = {},
                    onDownloads = {},
                    onTabs = {},
                    onMenu = {},
                    accentColor = MaterialTheme.colorScheme.primary,
                )
            }
        }

        composeRule.onNodeWithContentDescription("Forward").assertIsNotEnabled()
        composeRule.runOnIdle { canGoForward.value = true }
        composeRule.onNodeWithContentDescription("Forward").assertIsEnabled().performClick()
        composeRule.runOnIdle { assertEquals(1, forwardCalls) }
    }

    @Test
    fun tabsControlAnnouncesCurrentTabCount() {
        val tabCount = mutableStateOf(1)

        composeRule.setContent {
            MaterialTheme {
                BrowserBottomBar(
                    canGoBack = false,
                    canGoForward = false,
                    tabCount = tabCount.value,
                    ongoingDownloadCount = 0,
                    onBack = {},
                    onForward = {},
                    onHome = {},
                    onDownloads = {},
                    onTabs = {},
                    onMenu = {},
                    accentColor = MaterialTheme.colorScheme.primary,
                )
            }
        }

        composeRule.onNodeWithContentDescription("Tabs, 1 tab").assertExists()
        composeRule.runOnIdle { tabCount.value = 3 }
        composeRule.onNodeWithContentDescription("Tabs, 3 tabs").assertExists()
    }
}
