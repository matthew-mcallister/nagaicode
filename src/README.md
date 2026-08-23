# Design notes

## Event system

UI is using a classical hierarchical event-based architecture.

- Components own their children and manage their placement and layout
- Parent components pass input events and change notifications to their
  children
- Children communicate with their ancestors by bubbling events upward in
  response to input

## Rendering

The rendering code is a bit rough but is functional and row-based: each
component is a rectangle and "renders" itself by returning a list of rows to
display. Each row is a formatted sequence of text and terminal commands which
must be the exact width expected by the parent. Components must always be the
exact width expected by the parent, but parents have the option to accommodate
variable-height children.

## Async

The application runs on a tokio event loop. However, async is solely used for
networking-related background tasks to prevent blocking the UI. All other logic
is strictly single-threaded to ensure app behavior is as predictable and
race-free as possible.

# Testing

Terminal, network, and tool calls/shell commands are mocked in testing. All
reads/writes/requests are recorded and artificial data is returned.

(wip) A query system exists that allows high-level unit tests to probe deeply
nested application state. Any object which implements Queryable is able to
parse query URIs referencing and return a JSON representation of queried data
pulled from the object or its descendants.
