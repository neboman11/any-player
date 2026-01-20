# Contributing to Any Player

We welcome contributions! This guide will help you understand our development workflow, coding standards, and testing practices.

## Getting Started

Before contributing:

1. **Read the Documentation** - Familiarize yourself with [copilot-instructions.md](.github/copilot/copilot-instructions.md)
2. **Set up your development environment** - Follow the installation steps in the README

## Development Workflow

### Version Control

- Follow Semantic Versioning (currently 0.1.0)
- Versions synchronized between `package.json` and `Cargo.toml`
- Use Git for version control

### Branching Strategy

- Development happens on feature branches
- Main branch contains stable code
- Follow conventional commit messages

### Making Changes

1. Create a feature branch from main
2. Make changes following coding standards
3. Test thoroughly (unit tests + manual testing)
4. Submit pull request for review
5. Merge after approval

## Coding Standards

### Rust (Backend)

#### Naming Conventions

- `snake_case` for functions, variables, modules
- `PascalCase` for types and traits
- `SCREAMING_SNAKE_CASE` for constants

#### Code Organization

- Keep functions focused on single responsibilities
- Use the `MusicProvider` trait for all provider implementations
- Place business logic in separate modules (models, providers, playback)
- Expose public APIs through `mod.rs` files

#### Error Handling

- Use `Result<T, E>` for all fallible operations
- Use `anyhow::Result` in command handlers
- Create custom error types (e.g., `ProviderError`) for domain errors
- Use `?` operator for error propagation

#### State Management

- Use `Arc<Mutex<T>>` for shared mutable state
- Keep lock duration minimal to avoid deadlocks
- Use `tokio::sync::Mutex` for async code

#### Logging

- Use `tracing` crate with appropriate levels
- Include context in log messages
- Log security-relevant events

#### Documentation

- Use `///` comments for public items
- Document module purpose at the top of each file
- Include examples for complex public APIs
- Document error conditions and panics
- Follow the documentation style in [config/mod.rs](src-tauri/src/config/mod.rs) and [models/mod.rs](src-tauri/src/models/mod.rs)

### TypeScript/React (Frontend)

#### Naming Conventions

- `camelCase` for variables and functions
- `PascalCase` for types, interfaces, and React components
- Prefix custom hooks with `use` (e.g., `usePlayback`)

#### Component Structure

- Use functional components exclusively
- Use hooks for state management
- Memoize expensive computations with `useMemo`
- Memoize callbacks with `useCallback`
- Keep components focused and single-purpose

#### Type Safety

- Define all types in [types.ts](src/types.ts)
- Match Rust backend types exactly
- Use `interface` for object shapes
- Use `type` for unions and complex types

#### API Communication

- Use `TauriAPI` class from [api.ts](src/api.ts)
- All backend calls through `invoke` from `@tauri-apps/api/core`
- Handle errors with try-catch and log to console

#### Documentation

- Use JSDoc comments (`/** */`) for exported functions and classes
- Document function parameters and return types
- Include examples for complex APIs
- Follow the documentation style in [api.ts](src/api.ts)

## Testing

### Unit Testing (Rust)

Tests are placed in `#[cfg(test)]` modules at the end of each source file.

#### Naming Convention

```rust
test_<function_name>_<scenario>
```

#### Test Types

- `#[test]` for synchronous tests
- `#[tokio::test]` for async tests

#### Best Practices

- Test both success and error paths
- Mock external dependencies
- Use descriptive test names
- Test error handling explicitly

#### Example

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_restore_spotify_session_no_cache_or_tokens() {
        // Test implementation
    }
}
```

### Integration Testing

- Test complete command flows from frontend to backend
- Verify state management across components
- Test provider integrations with mock servers

### Running Tests

```bash
# Run Rust tests
cd src-tauri
cargo test

# Run TypeScript tests (if configured)
pnpm test
```

## Architecture Guidelines

### Maintain Separation of Concerns

- Backend handles all business logic, authentication, and audio playback
- Frontend handles UI and user interaction
- All communication through Tauri commands

### Provider Implementation

- New providers must implement the `MusicProvider` trait
- Register providers with ProviderRegistry
- Store provider-specific state in the provider implementation
- Use `async_trait` for async trait methods
- Return `ProviderError` for domain-specific errors

### State Management

- **Backend**: Use `Arc<Mutex<T>>` for shared state across async tasks
- **Frontend**: Use React hooks (useState, useEffect) for component state
- Custom hooks should encapsulate related state and logic

## Security Guidelines

- **Never** store credentials in plaintext
- Use keyring crate for secure token storage
- Validate all user inputs in Tauri commands
- Use parameterized queries for any database operations
- Follow OAuth 2.0 best practices for authentication flows
- Log security-relevant events using tracing crate

## Version Compatibility

When contributing, respect the exact versions of languages, frameworks, and libraries:

### Language Versions

- **Rust**: Edition 2021 (never use features beyond edition 2021)
- **TypeScript**: ~5.6.2 (never use features beyond TypeScript 5.6.2)
- **ECMAScript**: ES2020 target (never use features beyond ES2020)

### Framework Versions

- **Tauri**: Version 2
- **React**: ^18.3.1
- **Vite**: ^6.0.3
- **Tokio**: Version 1 with full features

### Key Libraries

Respect version constraints when generating code:

**Rust Backend:**
- rspotify: 0.12
- librespot-\*: 0.8.0
- rodio: 0.17
- symphonia: 0.5
- reqwest: 0.11
- serde/serde_json: 1.x
- tokio: 1.x
- tracing/tracing-subscriber: 0.1/0.3
- anyhow: 1.0
- keyring: 3.6

**TypeScript Frontend:**
- @tauri-apps/api: ^2
- @tauri-apps/plugin-opener: ^2
- @vitejs/plugin-react: ^4.3.4

## Code Consistency

When contributing:

1. **Always** scan for similar existing code first
2. **Match** the patterns, style, and conventions of existing code
3. **Prioritize** consistency with this codebase over general best practices
4. **Never** introduce new patterns without finding precedent in the existing code
5. **Respect** the established architecture and boundaries
6. **Use** only language and framework features available in the detected versions

## Pull Request Process

1. Ensure your code follows all coding standards
2. Add or update tests as needed
3. Update documentation if you're changing functionality
4. Ensure all tests pass (`cargo test` and `pnpm test`)
5. Write a clear PR description explaining your changes
6. Reference any related issues
7. Wait for code review and address feedback

## Questions?

If you have questions about contributing, please:

- Review the [copilot-instructions.md](.github/copilot/copilot-instructions.md) for detailed guidance
- Open an issue for discussion
- Reach out to maintainers

Thank you for contributing to Any Player!
