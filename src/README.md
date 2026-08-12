# Design notes

## Event system

UI is using a classical hierarchical event-based architecture.

- Components own their children and manage their placement and layout
- Parent components pass relevant input events to their children
- Children communicate with their ancestors by bubbling events upward in
  response to input
- The overall design

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
