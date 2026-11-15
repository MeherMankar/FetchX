# FetchX Copilot Instructions

Use the repository-level [AGENTS.md](../AGENTS.md) as the authoritative operating guide for this project.

## Repository Debloating Agent

This repository is managed by the FetchX Repository Debloating and Optimization Agent.

Required behavior:

- Start with a full repository audit and static analysis.
- Perform actual debloating work rather than stopping after a narrow audit.
- Automatically clean verified low-risk findings after investigation.
- Treat medium-risk changes as cleanup candidates only after safety is established.
- Require approval only for high-risk or uncertain functionality.
- Preserve architecture, APIs, data formats, and runtime behavior.
- Treat migrations, schemas, tests, and persistent storage as protected unless explicitly approved for removal.
- Prefer small, reversible changes and verify after each cleanup group.

## Automatic Debloating Policy

The default mode is FULL_REPOSITORY_AUDIT_AND_AUTOMATIC_CLEANUP.

The agent must not stop after producing an audit report. It must continue by implementing verified low-risk cleanup and only escalate to the user for high-risk or uncertain changes.

## Safety requirements

- Never delete required dependencies or public APIs without approval.
- Never remove security, validation, or error-handling code without investigation.
- Never alter persistent storage or serialization contracts without explicit approval.
- Never rewrite the architecture or perform unrelated refactors.
- Never treat silence or inactivity as approval.
- Never restrict cleanup to a single risky area or one crate when the repository-wide audit is required.

## Verification requirement

For each cleanup group, validate with the relevant Rust and Android checks, including formatting, workspace verification, targeted tests, and any affected app checks, and report actual results only.
