package dev.fetchx.browser

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.MutableState
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test

class TabGridSheetInstrumentedTest {
    @get:Rule
    val composeRule = createComposeRule()

    private val profiles = listOf(
        BrowserProfile("default", "Default", "#4F8EF7", "Default tabs"),
        BrowserProfile("work", "Work", "#7C6AF2", "Work tabs"),
    )

    @Test
    fun `empty state action meets minimum touch target height`() {
        showTabs(emptyList(), "missing")

        composeRule.onNodeWithText("Open new tab").assertHeightIsAtLeast(48.dp)
    }

    @Test
    fun `rendered cards expose stable IDs and exactly one active ID`() {
        val tabs = listOf(
            TabState("default-url", "https://example.test/five-tab-selected", "Neutral title"),
            TabState("default-title", "https://title.test", "Five-tab-selected title"),
            TabState(
                "work-url",
                "https://example.test/FIVE-TAB-SELECTED",
                "Other title",
                spaceId = "work",
            ),
        )
        showTabs(tabs, "work-url")

        listOf("default-url", "default-title", "work-url").forEach { id ->
            composeRule.onNodeWithTag("tab-card-$id").assertExists()
        }
        composeRule.onNodeWithTag("tab-card-work-url").assertIsSelected()
        composeRule.onNodeWithTag("tab-card-default-url").assertIsNotSelected()
        composeRule.onNodeWithTag("tab-card-default-title").assertIsNotSelected()
    }

    @Test
    fun `rendered search and space filters preserve URL title and tab identity semantics`() {
        val tabs = mutableStateOf(
            listOf(
                TabState("default-url", "https://example.test/five-tab-selected", "Neutral title"),
                TabState("default-title", "https://title.test", "Five-tab-selected title"),
                TabState(
                    "work-url",
                    "https://example.test/FIVE-TAB-SELECTED",
                    "Other title",
                    spaceId = "work",
                ),
                TabState("work-other", "https://work.test", "Unmatched", spaceId = "work"),
            ),
        )
        setContent(tabs, "work-url")
        typeSearch("five-tab-selected")

        listOf("default-url", "default-title", "work-url").forEach { id ->
            composeRule.onNodeWithTag("tab-card-$id").assertExists()
        }
        composeRule.onNodeWithTag("tab-card-work-other").assertDoesNotExist()

        composeRule.onNodeWithTag("tab-space-filter-default").performClick()
        composeRule.onNodeWithTag("tab-card-default-url").assertExists()
        composeRule.onNodeWithTag("tab-card-default-title").assertExists()
        composeRule.onNodeWithTag("tab-card-work-url").assertDoesNotExist()

        composeRule.onNodeWithTag("tab-space-filter-work").performClick()
        composeRule.onNodeWithTag("tab-card-work-url").assertExists()
        composeRule.onNodeWithTag("tab-card-default-url").assertDoesNotExist()

        composeRule.onNodeWithContentDescription("Close search").performClick()
        composeRule.onNodeWithTag("tab-card-work-url").assertExists()
        composeRule.onNodeWithTag("tab-card-work-other").assertExists()
        composeRule.onNodeWithTag("tab-card-default-url").assertDoesNotExist()

        composeRule.onNodeWithTag("tab-space-filter-all").performClick()
        composeRule.onNodeWithTag("tab-card-default-url").assertExists()
        composeRule.onNodeWithTag("tab-card-work-url").assertExists()

        typeSearch("five-tab-selected")
        composeRule.runOnIdle {
            tabs.value = listOf(
                tabs.value.first(),
                TabState("replacement", "https://replacement.test", "FIVE-TAB-SELECTED replacement"),
                tabs.value[2],
                tabs.value[3],
            )
        }
        composeRule.waitForIdle()
        composeRule.onNodeWithTag("tab-card-default-title").assertDoesNotExist()
        composeRule.onNodeWithTag("tab-card-replacement").assertExists()
        composeRule.onNodeWithTag("tab-card-work-url").assertExists()
        composeRule.onNodeWithTag("tab-card-work-url").assertIsSelected()
    }

    @Test
    fun `search observes URL after the tab state update`() {
        val tabs = mutableStateOf(
            listOf(TabState("navigating", "about:blank", "New tab")),
        )
        setContent(tabs, "navigating")
        typeSearch("five-tab-selected")
        composeRule.onNodeWithTag("tab-card-navigating").assertDoesNotExist()

        composeRule.runOnIdle {
            tabs.value = listOf(
                tabs.value.single().copy(url = "https://example.test/five-tab-selected"),
            )
        }
        composeRule.waitForIdle()
        composeRule.onNodeWithTag("tab-card-navigating").assertExists()
    }

    private fun showTabs(tabs: List<TabState>, activeTabId: String) {
        setContent(mutableStateOf(tabs), activeTabId)
    }

    private fun setContent(
        tabs: MutableState<List<TabState>>,
        activeTabId: String,
    ) {
        composeRule.setContent {
            val currentTabs = tabs.value
            MaterialTheme {
                TabGridSheet(
                    tabs = currentTabs,
                    activeTabId = activeTabId,
                    profiles = profiles,
                    selectedProfileId = "default",
                    onSelect = {},
                    onClose = {},
                    onCloseAll = { _ -> },
                    onNewTab = {},
                    onDismiss = {},
                    onOpenBrowserMenu = {},
                )
            }
        }
        composeRule.waitForIdle()
    }

    private fun typeSearch(query: String) {
        composeRule.onNodeWithContentDescription("Search tabs").performClick()
        composeRule.onNodeWithTag("tab-grid-search").performTextInput(query)
        composeRule.waitForIdle()
    }
}
