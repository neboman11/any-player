# GitHub Copilot Instructions

## Priority Guidelines

When generating code for this repository:

1. **Version Compatibility**: Always detect and respect the exact versions of languages, frameworks, and libraries used in this project
2. **Context Files**: Prioritize patterns and standards defined in the .github/copilot directory
3. **Codebase Patterns**: When context files don't provide specific guidance, scan the codebase for established patterns
4. **Architectural Consistency**: Maintain the established Tauri-based architecture with clear separation between frontend and backend
5. **Code Quality**: Prioritize maintainability, security, and testability in all generated code

## Technology Version Detection

Before generating code, scan the codebase to identify:

1. **Language Versions**: Detect the exact versions of programming languages in use
   - **Rust**: Edition 2021 (as specified in [Cargo.toml](../../src-tauri/Cargo.toml))
   - **TypeScript**: ~5.6.2 (as specified in [package.json](../../package.json))
   - **ECMAScript**: ES2020 target (as specified in [tsconfig.json](../../tsconfig.json))
   - Never use Rust language features beyond edition 2021
   - Never use TypeScript or ECMAScript features beyond their detected versions

2. **Framework Versions**: Identify the exact versions of all frameworks
   - **Tauri**: Version 2 (as specified in Cargo.toml and package.json)
   - **React**: ^18.3.1 (as specified in package.json)
   - **Vite**: ^6.0.3 (as specified in package.json)
   - **Tokio**: Version 1 with full features (as specified in Cargo.toml)
   - Respect version constraints when generating code
   - Never suggest features not available in the detected framework versions

3. **Library Versions**: Note the exact versions of key libraries and dependencies
   - **Rust Backend**:
     - rspotify: 0.12 (Spotify Web API)
     - librespot-\*: 0.8.0 (Spotify streaming)
     - rodio: 0.17 (audio playback)
     - symphonia: 0.5 (audio decoding)
     - reqwest: 0.11 (HTTP client)
     - serde/serde_json: 1.x (serialization)
     - tokio: 1.x (async runtime)
     - tracing/tracing-subscriber: 0.1/0.3 (logging)
     - anyhow: 1.0 (error handling)
     - keyring: 3.6 (secure token storage)
   - **TypeScript Frontend**:
     - @tauri-apps/api: ^2
     - @tauri-apps/plugin-opener: ^2
     - @vitejs/plugin-react: ^4.3.4
   - Generate code compatible with these specific versions
   - Never use APIs or features not available in the detected versions

## Codebase Scanning Instructions

When context files don't provide specific guidance:

1. Identify similar files to the one being modified or created
2. Analyze patterns for:
   - Naming conventions (snake_case for Rust, camelCase for TypeScript)
   - Code organization (modules in Rust, components/hooks in React)
   - Error handling (Result<T, E> in Rust, try-catch with error logging in TypeScript)
   - Logging approaches (tracing crate in Rust, console.error/log in TypeScript)
   - Documentation style (/// comments in Rust, JSDoc in TypeScript)
   - Testing patterns (#[test] and #[tokio::test] in Rust)
3. Follow the most consistent patterns found in the codebase
4. When conflicting patterns exist, prioritize patterns in core modules (lib.rs, commands.rs, api.ts)
5. Never introduce patterns not found in the existing codebase

## Architecture Overview

This is a **Tauri 2** application with a clear separation between frontend and backend:

### Backend (Rust - src-tauri/)

- **Entry Point**: [lib.rs](../../src-tauri/src/lib.rs) - Application initialization and module organization
- **Commands**: [commands.rs](../../src-tauri/src/commands.rs) - Tauri command handlers (API layer between frontend and backend)
- **Models**: [models/mod.rs](../../src-tauri/src/models/mod.rs) - Core data structures (Track, Playlist, Source, PlaybackState, RepeatMode)
- **Providers**: [providers/mod.rs](../../src-tauri/src/providers/mod.rs) - Music provider trait and implementations (Spotify, Jellyfin)
- **Playback**: [playback/mod.rs](../../src-tauri/src/playback/mod.rs) - Audio playback management with rodio and librespot
- **Config**: [config/mod.rs](../../src-tauri/src/config/mod.rs) - Configuration and secure token storage using keyring

### Frontend (TypeScript/React - src/)

- **Entry Point**: [App.tsx](../../src/App.tsx) - Main application component with page routing
- **API Layer**: [api.ts](../../src/api.ts) - TauriAPI class wrapping all invoke commands
- **Types**: [types.ts](../../src/types.ts) - TypeScript type definitions matching Rust models
- **Components**: [components/](../../src/components/) - React UI components
- **Hooks**: [hooks/](../../src/hooks/) - Custom React hooks for state management and API interaction

### Key Architectural Patterns

1. **Provider Pattern**: Music providers (Spotify, Jellyfin) implement the `MusicProvider` trait
2. **State Management**: Backend manages state with Arc<Mutex<T>>, frontend uses React hooks
3. **Command Pattern**: All frontend-to-backend communication uses Tauri commands defined in commands.rs
4. **Async/Await**: Both frontend and backend use async/await for asynchronous operations
5. **Token Security**: OAuth tokens stored in OS-specific secure storage via keyring crate

## Code Quality Standards

### Maintainability

- Write self-documenting code with clear naming
- **Rust Naming**:
  - Use snake_case for functions, variables, and modules
  - Use PascalCase for types and traits
  - Use SCREAMING_SNAKE_CASE for constants
- **TypeScript Naming**:
  - Use camelCase for variables and functions
  - Use PascalCase for types, interfaces, and React components
  - Prefix custom hooks with "use" (e.g., usePlayback, useSpotifyAuth)
- Keep functions focused on single responsibilities
- Limit function complexity (avoid deeply nested logic)

### Performance

- Use Arc<Mutex<T>> for shared state in Rust backend
- Implement memoization in React (useMemo, useCallback) as seen in [App.tsx](../../src/App.tsx)
- Cache API responses where appropriate
- Use async/await for I/O operations
- Leverage Tokio runtime efficiently with proper async boundaries

### Security

- **Never** store tokens in plain text files
- Use keyring crate for secure token storage (see [config/mod.rs](../../src-tauri/src/config/mod.rs))
- Validate all user inputs in Tauri commands
- Use parameterized queries for any database operations
- Follow OAuth 2.0 best practices for authentication flows
- Log security-relevant events using tracing crate

### Testability

- Write unit tests for business logic using #[test] and #[tokio::test]
- Follow test patterns in [config/mod.rs](../../src-tauri/src/config/mod.rs) and [providers/mod.rs](../../src-tauri/src/providers/mod.rs)
- Use descriptive test names (e.g., test_restore_spotify_session_no_cache_or_tokens)
- Mock external dependencies in tests
- Test error handling paths

## Documentation Requirements

### Rust Documentation

- Use triple-slash comments (///) for public items
- Document module purpose at the top of each file
- Include examples for complex public APIs
- Document error conditions and panics
- Follow the documentation style in [config/mod.rs](../../src-tauri/src/config/mod.rs) and [models/mod.rs](../../src-tauri/src/models/mod.rs)

### TypeScript Documentation

- Use JSDoc comments (/\*\* \*/) for exported functions and classes
- Document function parameters and return types
- Include examples for complex APIs
- Follow the documentation style in [api.ts](../../src/api.ts)

## Testing Approach

### Unit Testing (Rust)

- Place tests in a #[cfg(test)] mod tests block at the end of each module
- Use #[test] for synchronous tests
- Use #[tokio::test] for async tests
- Follow naming: test*<function_name>*<scenario>
- Test both success and error paths
- Mock external dependencies
- Examples: See test sections in [config/mod.rs](../../src-tauri/src/config/mod.rs) and [providers/mod.rs](../../src-tauri/src/providers/mod.rs)

### Integration Testing

- Test full command flows from frontend to backend
- Verify state management across components
- Test provider integrations

## Technology-Specific Guidelines

### Rust Backend Guidelines

- **Edition**: Strictly use Rust 2021 edition features only
- **Async Runtime**: Use tokio with full features for all async operations
- **Error Handling**:
  - Use Result<T, E> for fallible operations
  - Use anyhow::Result for command handlers
  - Create custom error types (e.g., ProviderError) for domain-specific errors
  - Use ? operator for error propagation
- **Logging**: Use tracing crate with appropriate levels (trace, debug, info, warn, error)
- **Serialization**: Use serde with derive macros for all data structures crossing the FFI boundary
- **State Management**:
  - Use Arc<Mutex<T>> for shared mutable state
  - Keep locks short-lived to avoid deadlocks
  - Use tokio::sync::Mutex for async code
- **Tauri Commands**:
  - Mark command handlers with #[tauri::command]
  - Accept State<'\_, AppState> for accessing application state
  - Return Result<T, String> for all commands

### TypeScript/React Guidelines

- **TypeScript Version**: Use TypeScript 5.6.2 features
- **ECMAScript**: Target ES2020 (no newer features)
- **Strict Mode**: Enable all strict TypeScript checks
- **React Patterns**:
  - Use functional components exclusively (no class components)
  - Use hooks for state management (useState, useEffect, useCallback, useMemo)
  - Custom hooks should start with "use" prefix
  - Memoize expensive computations with useMemo
  - Memoize callbacks passed to children with useCallback
- **Type Definitions**:
  - Define all types in [types.ts](../../src/types.ts)
  - Use interface for object shapes
  - Use type for unions, primitives, and complex types
  - Match backend Rust types exactly
- **API Calls**:
  - Use TauriAPI class from [api.ts](../../src/api.ts)
  - All Tauri commands go through invoke from @tauri-apps/api/core
  - Handle errors with try-catch and log to console
- **Component Structure**:
  - Export components as named exports from component files
  - Re-export from [components/index.ts](../../src/components/index.ts)
  - Keep components focused and single-purpose

### Vite Configuration

- Follow patterns in [vite.config.ts](../../vite.config.ts)
- Server port: 1420 (strict)
- HMR port: 1421
- Ignore src-tauri directory in watch
- Use @vitejs/plugin-react

## General Best Practices

### Error Handling

- **Rust**: Always use Result types, never panic in production code
- **TypeScript**: Use try-catch for async operations, log errors to console
- Provide meaningful error messages
- Log errors with appropriate context using tracing (Rust) or console.error (TypeScript)

### Logging

- **Rust**: Use tracing crate with structured logging
  - Level TRACE for detailed execution flow
  - Level DEBUG for development information
  - Level INFO for important state changes
  - Level WARN for recoverable errors
  - Level ERROR for critical failures
- **TypeScript**: Use console for client-side logging
  - console.log for general information
  - console.error for errors
  - Include context in log messages

### Code Organization

- Keep modules focused and cohesive
- Separate concerns (data models, business logic, UI)
- Use the established folder structure
- Export public APIs through index files (mod.rs, index.ts)

### Naming Conventions

- **Rust Files**: snake_case.rs
- **TypeScript Files**: PascalCase.tsx for components, camelCase.ts for utilities
- **Constants**: SCREAMING_SNAKE_CASE in both languages
- **Type Names**: PascalCase in both languages
- Match the style of surrounding code

## Project-Specific Guidance

### Music Provider Implementation

- All providers must implement the `MusicProvider` trait
- Register providers with ProviderRegistry
- Store provider-specific state in the provider implementation
- Use async_trait for async trait methods
- Return ProviderError for domain-specific errors

### Playback Management

- PlaybackManager coordinates between providers and audio output
- Use rodio for local audio playback
- Use librespot for Spotify premium streaming
- Manage playback state (Playing, Paused, Stopped)
- Support shuffle and repeat modes (Off, One, All)

### Authentication Flows

- OAuth 2.0 for Spotify (with local callback server)
- API key authentication for Jellyfin
- Store tokens securely using keyring crate
- Support token refresh for expired credentials
- Clear pattern: restore_spotify_session() on app startup

### Command Handlers

- All frontend-to-backend communication uses Tauri commands
- Commands defined in [commands.rs](../../src-tauri/src/commands.rs)
- Commands access shared state via State<'\_, AppState>
- Return serializable types (implement Serialize/Deserialize)
- Handle all errors and convert to String for cross-boundary compatibility

### State Management

- **Backend**: Arc<Mutex<>> for shared state across async tasks
- **Frontend**: React hooks (useState, useEffect) for component state
- Custom hooks encapsulate related state and logic
- Poll for playback status updates using intervals

## Version Control Guidelines

- Follow Semantic Versioning (currently at 0.1.0)
- Version specified in both [package.json](../../package.json) and [Cargo.toml](../../src-tauri/Cargo.toml)
- Keep versions synchronized between frontend and backend

## Important: Consistency Over External Best Practices

When generating code:

1. **Always** scan for similar existing code first
2. **Match** the patterns, style, and conventions of existing code
3. **Prioritize** consistency with this codebase over general best practices
4. **Never** introduce new patterns without finding precedent in the existing code
5. **Respect** the established architecture and boundaries
6. **Use** only language and framework features available in the detected versions
