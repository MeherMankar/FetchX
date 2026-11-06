package dev.fetchx.browser

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performImeAction
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class FindInPageInstrumentedTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun typingQueryUpdatesSearchRequest() {
        var query = ""

        composeRule.setContent {
            MaterialTheme {
                FindInPagePanel(
                    state = FindInPageState("tab-1", "https://example.com"),
                    focusRequester = FocusRequester(),
                    onQueryChange = { query = it },
                    onPrevious = {},
                    onNext = {},
                    onClear = {},
                    onClose = {},
                )
            }
        }

        composeRule.onNodeWithText("Search this page").performTextInput("FetchX")
        composeRule.runOnIdle { assertEquals("FetchX", query) }
    }

    @Test
    fun queryUpdatesDisplayedMatchCount() {
        val state = mutableStateOf(
            FindInPageState(
                tabId = "tab-1",
                pageUrl = "https://example.com",
                query = "FetchX",
                currentMatch = 2,
                totalMatches = 4,
                found = true,
            ),
        )

        composeRule.setContent {
            MaterialTheme {
                FindInPagePanel(
                    state = state.value,
                    focusRequester = FocusRequester(),
                    onQueryChange = {},
                    onPrevious = {},
                    onNext = {},
                    onClear = {},
                    onClose = {},
                )
            }
        }

        composeRule.onNodeWithText("2 / 4").assertExists()
        composeRule.runOnIdle {
            state.value = state.value.copy(currentMatch = 3, totalMatches = 4, found = true)
        }
        composeRule.onNodeWithText("3 / 4").assertExists()
    }

    @Test
    fun imeSearchAdvancesOnceToNextMatch() {
        var nextCalls = 0

        composeRule.setContent {
            MaterialTheme {
                FindInPagePanel(
                    state = FindInPageState(
                        tabId = "tab-1",
                        pageUrl = "https://example.com",
                        query = "FetchX",
                        currentMatch = 1,
                        totalMatches = 3,
                        found = true,
                    ),
                    focusRequester = FocusRequester(),
                    onQueryChange = {},
                    onPrevious = {},
                    onNext = { nextCalls++ },
                    onClear = {},
                    onClose = {},
                )
            }
        }

        composeRule.onNodeWithText("FetchX").performImeAction()
        composeRule.runOnIdle { assertEquals(1, nextCalls) }
    }

    @Test
    fun nextAndPreviousAdvanceWhenMatchesExist() {
        var previousCalls = 0
        var nextCalls = 0

        composeRule.setContent {
            MaterialTheme {
                FindInPagePanel(
                    state = FindInPageState(
                        tabId = "tab-1",
                        pageUrl = "https://example.com",
                        query = "FetchX",
                        currentMatch = 2,
                        totalMatches = 3,
                        found = true,
                    ),
                    focusRequester = FocusRequester(),
                    onQueryChange = {},
                    onPrevious = { previousCalls++ },
                    onNext = { nextCalls++ },
                    onClear = {},
                    onClose = {},
                )
            }
        }

        composeRule.onNodeWithContentDescription("Previous match").assertIsEnabled().performClick()
        composeRule.onNodeWithContentDescription("Next match").assertIsEnabled().performClick()
        composeRule.runOnIdle {
            assertEquals(1, previousCalls)
            assertEquals(1, nextCalls)
        }
    }

    @Test
    fun zeroMatchesAndEmptyQueryDisableNavigation() {
        composeRule.setContent {
            MaterialTheme {
                FindInPagePanel(
                    state = FindInPageState(
                        tabId = "tab-1",
                        pageUrl = "https://example.com",
                        query = "",
                        totalMatches = 0,
                    ),
                    focusRequester = FocusRequester(),
                    onQueryChange = {},
                    onPrevious = {},
                    onNext = {},
                    onClear = {},
                    onClose = {},
                )
            }
        }

        composeRule.onNodeWithContentDescription("Previous match").assertIsNotEnabled()
        composeRule.onNodeWithContentDescription("Next match").assertIsNotEnabled()

        composeRule.setContent {
            MaterialTheme {
                FindInPagePanel(
                    state = FindInPageState(
                        tabId = "tab-1",
                        pageUrl = "https://example.com",
                        query = "missing",
                        totalMatches = 0,
                        found = false,
                    ),
                    focusRequester = FocusRequester(),
                    onQueryChange = {},
                    onPrevious = {},
                    onNext = {},
                    onClear = {},
                    onClose = {},
                )
            }
        }

        composeRule.onNodeWithContentDescription("Previous match").assertIsNotEnabled()
        composeRule.onNodeWithContentDescription("Next match").assertIsNotEnabled()
    }

    @Test
    fun clearQueryRemovesResultsAndCloseInvokesCloseAction() {
        val state = mutableStateOf(
            FindInPageState(
                tabId = "tab-1",
                pageUrl = "https://example.com",
                query = "FetchX",
                currentMatch = 1,
                totalMatches = 3,
                found = true,
            ),
        )
        var closeCalls = 0

        composeRule.setContent {
            MaterialTheme {
                FindInPagePanel(
                    state = state.value,
                    focusRequester = FocusRequester(),
                    onQueryChange = {},
                    onPrevious = {},
                    onNext = {},
                    onClear = {
                        state.value = state.value.copy(
                            query = "",
                            currentMatch = 0,
                            totalMatches = 0,
                            found = false,
                        )
                    },
                    onClose = { closeCalls++ },
                )
            }
        }

        composeRule.onNodeWithContentDescription("Clear search").performClick()
        composeRule.onNodeWithText("1 / 3").assertDoesNotExist()
        composeRule.onNodeWithContentDescription("Previous match").assertIsNotEnabled()
        composeRule.onNodeWithContentDescription("Next match").assertIsNotEnabled()

        composeRule.onNodeWithContentDescription("Close Find in Page").performClick()
        composeRule.runOnIdle { assertEquals(1, closeCalls) }
    }
}
