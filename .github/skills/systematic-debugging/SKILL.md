---
name: systematic-debugging
description: 'Use to diagnose bugs, crashes, failing tests, or unexpected application behavior. Reproduce the issue, gather evidence, test a falsifiable hypothesis, fix the root cause, and verify with a regression test.'
---

# Systematic Debugging

## Workflow

1. Reproduce the failure with the narrowest reliable scenario. Capture the exact input, environment, expected behavior, and observed result.
2. Read the relevant logs, stack traces, state transitions, and nearby implementation. Follow the execution path instead of guessing from symptoms.
3. Form one falsifiable hypothesis at a time. Use the least intrusive diagnostic that can confirm or reject it; avoid broad logging or debugger changes.
4. Change the underlying cause, not just the visible symptom. Keep the patch focused and preserve unrelated work.
5. Add or update a regression test that would fail before the fix.
6. Run the smallest relevant test, then the narrowest build or integration check needed. Report checks that could not be run.
7. Remove temporary diagnostics and debug-only settings before finishing.

## Android and JVM Notes

- Prefer reproducible local or emulator scenarios and filtered `adb logcat` output over speculative UI diagnosis.
- Verify both app state and external effects, such as persisted records and files; a queued UI action alone does not prove completion.
- If attaching a JVM debugger, bind JDWP to loopback, scope it to the target process, and never leave it enabled in production.

## Source

Adapted from the [Awesome Copilot Java debugging skill](https://github.com/github/awesome-copilot/tree/main/skills/java-debugging).
