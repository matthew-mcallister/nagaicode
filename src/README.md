# Design notes

## Project structure

- `migrations/`: migrations
- `src/`
  - `src/README.md`: architectural notes
  - `src/agent.rs`: LLM agents
  - `src/app.rs`: main entrypoint
  - `src/task.rs`: task spawning and lifecycle
  - `src/query.rs`: debug query trait
  - `src/session.rs`: session, turn, and response tables
  - `src/item.rs`: item table
  - `src/testing.rs`: unit test helpers
  - `src/tools/`:
    - `src/tools/mod.rs`: tool registry and task definition
  - `src/interface/`
    - `src/interface/mod.rs`: inference API interface
  - `src/ui/`
    - `src/ui/component.rs`: UI component trait

## Event system

UI is using a classical hierarchical event-based architecture.

- Components own their children and manage their placement and layout
- Parent components pass input events and change notifications to their
  children
- Children communicate with their ancestors by bubbling events upward in
  response to input

Async tasks and agents can also trigger events by sending them to the app
through a global MPSC channel. Basically, AppEvent serves as a funnel through
which all global effects are processed. This makes the app digestible as local
component state + global state machine powered by AppEvent, with a DB for
persistence.

## Rendering

The rendering code is a bit rough but is functional and row-based: each
component is a rectangle and "renders" itself by returning a list of rows to
display. Each row is a formatted sequence of text and terminal commands which
must be the exact width expected by the parent. Components must always be the
exact width expected by the parent, but parents have the option to accommodate
variable-height children.

## Async

The application runs on a tokio event loop. However, async is solely used for
background tasks to prevent blocking the UI. All UI logic is strictly
single-threaded to ensure app behavior is as predictable and race-free as
possible.

# Testing

Terminal, network, and tool calls/shell commands are mocked in testing. All
reads/writes/requests are recorded and artificial data is returned.

A query system exists that allows high-level unit tests to probe deeply nested
application state. Any object which implements DataQuery is able to parse query
URIs referencing and return a JSON representation of queried data pulled from
the object or its descendants.

# Arenas

An arena data structure with generational IDs is implemented in `arena.rs`. It
is used for linked list data structures. At some point it is supposed to
become a standalone crate but currently it is copy-pasted in.
