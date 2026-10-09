# AI contribution rules

- Rust owns all business logic.
- The frontend must never contain download business logic.
- Every engine must use a defined abstraction.
- Do not duplicate engine functionality.
- Do not concatenate shell commands.
- External processes must receive structured arguments.
- Never store secrets in source code.
- Persistent state must go through the storage layer.
- New functionality must have tests.
- Do not modify unrelated modules.
- Do not introduce dependencies without justification.
- Prefer small, testable modules.
- Use async Rust with Tokio where asynchronous work is required.
- Use `Result`-based error handling.
- Avoid `unwrap()` and `expect()` in production code unless there is a documented invariant.
- Do not implement DRM circumvention or access-control bypass functionality.
- Do not collect credentials or authentication data without explicit user-authorised functionality.
- Run formatting, linting, and tests before declaring a task complete.
