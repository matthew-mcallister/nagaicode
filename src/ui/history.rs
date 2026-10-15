//! Scrolling chat history. Data is constructed as a linked list of rows for
//! fast viewport scrolling and rendering.

use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use fnv::FnvHashMap;
use serde_json::{Value, json};

use crate::arena::{Arena, Id};
use crate::item::Item;
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::tools::ToolRegistry;
use crate::ui::Component;
use crate::ui::canvas::Canvas;
use crate::ui::UiContext;
use crate::ui::render_item::{
    CommandOutputRenderItem, CommandPromptRenderItem, ErrorRenderItem, HelpRenderItem,
    RenderItem, get_item_content,
};
use crate::ui::markdown::ResumePoint;
use crate::ui::style::Theme;
use crate::ui::styled_string::StyledString;

#[derive(Debug)]
struct Viewport {
    top: Id<HistoryRow>,
    bottom: Id<HistoryRow>,
    /// Absolute row of viewport top
    top_row: usize,
    /// Absolute row of viewport bottom
    bottom_row: usize,
    // Absolute row of item containing viewport bottom
    bottom_item_row: usize,
}

#[derive(Debug)]
pub struct HistoryRow {
    item: Id<HistoryItem>,
    /// Preformatted, pre-padded row contents
    preformatted: StyledString,
    prev: Id<HistoryRow>,
    next: Id<HistoryRow>,
}

fn row_offset(
    rows: &Arena<HistoryRow>,
    head: Id<HistoryRow>,
    base: Id<HistoryRow>,
    offset: isize,
) -> Option<Id<HistoryRow>> {
    let mut row = base;
    if offset >= 0 {
        for _ in 0..offset {
            row = rows[row].next;
            if row == head {
                return None;
            }
        }
    } else {
        for _ in 0..-offset {
            row = rows[row].prev;
            if row == head {
                return None;
            }
        }
    }
    Some(row)
}

/// Deletes and unlinks rows from the arena
fn remove_rows(rows: &mut Arena<HistoryRow>, prev: Id<HistoryRow>, last: Id<HistoryRow>) {
    loop {
        let next = rows[prev].next;
        debug_assert!(rows[next].item != Id::null(), "removed head");
        rows[prev].next = rows[next].next;
        rows.remove(next);
        if next == last {
            break;
        }
    }
}

// Inserts and links rows
fn insert_rows(
    rows: &mut Arena<HistoryRow>,
    prev: Id<HistoryRow>,
    item: Id<HistoryItem>,
    rendered: Vec<StyledString>,
) {
    let next = rows[prev].next;
    let mut last = prev;
    for preformatted in rendered {
        let id = rows.insert(HistoryRow {
            item,
            preformatted,
            prev: last,
            next,
        });
        rows[last].next = id;
        last = id;
    }
    rows[next].prev = last;
}

#[derive(Debug)]
pub struct HistoryItem {
    content: Box<dyn RenderItem>,
    resume_point: ResumePoint,
    item_id: Option<i32>,
    /// Every item either comes from the DB (and has a real seqno) or is
    /// temporary and inherits the seqno of the most recent item before it.
    seqno: i64,
    /// Offset relative to first item with same seqno.
    seq_offset: u32,
    first_row: Id<HistoryRow>,
    last_row: Id<HistoryRow>,
    num_rows: usize,
}

impl HistoryItem {
    fn sort_key(&self) -> (i64, u32) {
        (self.seqno, self.seq_offset)
    }

    fn render(
        theme: &Theme,
        width: usize,
        content: &dyn RenderItem,
        resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        let (mut rendered, resume_point) = content.render(theme, width, resume);

        // TODO: smarter padding system
        if content.trailing_padding() {
            // Add vertical padding row
            let mut padding = StyledString::new(theme.base_style(), width);
            padding.pad_to_width(width);
            rendered.push(padding);
        }

        if rendered.is_empty() {
            // Every item must have at least one row. This keeps the list data
            // structure a lot simpler. Items that have no actual content are
            // ordinarily not inserted into the history in the first place.
            // This check handles pathological cases and bugs.
            rendered.push(StyledString::new(theme.base_style(), 0));
        }

        (rendered, resume_point)
    }

    fn create(
        theme: &Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        content: Box<dyn RenderItem>,
        item_id: Option<i32>,
    ) -> Id<Self> {
        let (rendered, resume_point) = Self::render(theme, width, &*content, Default::default());

        let item = items.insert(Self {
            content,
            resume_point,
            item_id,
            seqno: 0,
            seq_offset: 0,
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        });

        let next = rows[prev].next;
        let num_rows = rendered.len();
        insert_rows(rows, prev, item, rendered);
        items[item].first_row = rows[prev].next;
        items[item].last_row = rows[next].prev;
        items[item].num_rows = num_rows;

        item
    }

    // Updates and re-renders an existing item.
    //
    // Incremental rerenders are handled by passing a resume point to the
    // renderer to get a partial render, then overwriting all the rows past the
    // resume point.
    pub fn update(
        &mut self,
        theme: &Theme,
        rows: &mut Arena<HistoryRow>,
        head: Id<HistoryRow>,
        width: usize,
        content: Box<dyn RenderItem>,
    ) {
        let item_id = rows[self.first_row].item;  // Bit of a hack

        self.content = content;
        let (rendered, new_resume_point) = Self::render(theme, width, &*self.content, self.resume_point);
        debug_assert!(!rendered.is_empty());

        let prev = rows[self.first_row].prev;
        let next = rows[self.last_row].next;
        let len = self.resume_point.row + rendered.len();

        // FIXME: this iterates from end but should iterate in whichever
        // direction is shorter
        let offset = self.num_rows - self.resume_point.row;
        let insert_prev = row_offset(rows, head, self.last_row, -(offset as isize)).unwrap_or(head);
        remove_rows(rows, insert_prev, self.last_row);
        insert_rows(rows, insert_prev, item_id, rendered);

        self.first_row = rows[prev].next;
        self.last_row = rows[next].prev;
        self.num_rows = len;
        self.resume_point = new_resume_point;
    }
}

impl DataQuery for HistoryItem {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "content": self.query("/content")?,
                "resume_point": self.query("/resume_point")?,
                "item_id": self.query("/item_id")?,
                "seqno": self.query("/seqno")?,
                "seq_offset": self.query("/seq_offset")?,
                "sort_key": self.query("/sort_key")?,
                "first_row": self.query("/first_row")?,
                "last_row": self.query("/last_row")?,
                "num_rows": self.query("/num_rows")?,
            }))),
            "content" => Ok(QueryField::DataQuery(&*self.content)),
            "resume_point" => Ok(QueryField::DataQuery(&self.resume_point)),
            "item_id" => Ok(QueryField::Value(json!(self.item_id))),
            "seqno" => Ok(QueryField::Value(json!(self.seqno))),
            "seq_offset" => Ok(QueryField::Value(json!(self.seq_offset))),
            "sort_key" => Ok(QueryField::Value(json!(self.sort_key()))),
            "first_row" => Ok(QueryField::Value(self.first_row.to_json())),
            "last_row" => Ok(QueryField::Value(self.last_row.to_json())),
            "num_rows" => Ok(QueryField::Value(json!(self.num_rows))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Debug)]
pub struct History {
    item: Arena<HistoryItem>,
    rows: Arena<HistoryRow>,
    width: usize,
    theme: &'static Theme,
    /// Maximum viewport size
    max_height: usize,
    /// Head of circularly linked list. Contains no real data.
    head: Id<HistoryRow>,
    viewport: Viewport,
    /// Maps an `Item` id to the history item rendering it, so that
    /// `ItemUpdated` events can locate and rerender the right item.
    by_item_id: FnvHashMap<i32, Id<HistoryItem>>,
    tools: Arc<ToolRegistry>,
}

impl History {
    pub fn new(ctx: &UiContext, width: usize, max_height: usize, theme: &'static Theme) -> Self {
        let item = Arena::new();
        let mut rows = Arena::new();

        // Insert dummy head, distinct from all other rows.
        let head = rows.insert(HistoryRow {
            item: Id::null(),
            preformatted: StyledString::new(theme.base_style(), 0),
            prev: Id::null(),
            next: Id::null(),
        });
        rows[head].prev = head;
        rows[head].next = head;

        Self {
            item,
            rows,
            width,
            theme,
            max_height,
            head,
            viewport: Viewport {
                top: head,
                top_row: 0,
                bottom: head,
                bottom_row: 0,
                bottom_item_row: 0,
            },
            by_item_id: FnvHashMap::default(),
            tools: ctx.tools().clone(),
        }
    }

    /// Returns the tool registry.
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
    }

    pub fn num_rows(&self) -> usize {
        // Subtract header node
        self.rows.len() - 1
    }

    fn item_ids(&self) -> Vec<Id<HistoryItem>> {
        let mut items: Vec<Id<HistoryItem>> = Vec::new();
        for (_, row) in self.iter_range(self.head, self.last_row()) {
            let item = row.item;
            if items.last() != Some(&item) {
                items.push(item);
            }
        }
        items
    }

    fn first_row(&self) -> Id<HistoryRow> {
        self.rows[self.head].next
    }

    fn last_row(&self) -> Id<HistoryRow> {
        self.rows[self.head].prev
    }

    fn first_item(&self) -> Option<Id<HistoryItem>> {
        (self.num_rows() > 0).then(|| self.rows[self.first_row()].item)
    }

    fn last_item(&self) -> Option<Id<HistoryItem>> {
        (self.num_rows() > 0).then(|| self.rows[self.last_row()].item)
    }

    fn bottom_item(&self) -> Option<Id<HistoryItem>> {
        (self.viewport.bottom != self.head).then(|| self.rows[self.viewport.bottom].item)
    }

    /// Iterate over a range of rows. `prev` is not inclusive; `last` is
    /// inclusive.
    fn iter_range<'a>(&'a self, prev: Id<HistoryRow>, last: Id<HistoryRow>) -> HistoryRowIter<'a> {
        HistoryRowIter {
            rows: &self.rows,
            prev,
            last,
        }
    }

    /// O(n) row lookup relative to base row. Returns None if the offset is
    /// out of bounds.
    fn row_offset(&self, base: Id<HistoryRow>, offset: isize) -> Option<Id<HistoryRow>> {
        row_offset(&self.rows, self.head, base, offset)
    }

    fn less_than(&self, item: Id<HistoryItem>, other: Id<HistoryItem>) -> bool {
        self.item[item].sort_key() < self.item[other].sort_key()
    }

    fn item_row_diff(&self, from: Id<HistoryItem>, to: Id<HistoryItem>) -> usize {
        let mut rows = 0;
        let mut item = from;
        while item != to {
            rows += self.item[item].num_rows;
            item = self.rows[self.rows[self.item[item].last_row].next].item;
        }
        rows
    }

    /// O(n) row distance relative to base. base must come before other.
    /// Unspecified result if base comes after other.
    #[cfg(test)]
    fn row_diff(&self, base: Id<HistoryRow>, other: Id<HistoryRow>) -> isize {
        let mut row = base;
        let mut diff = 0;
        while row != other {
            row = self.rows[row].next;
            diff += 1;
        }
        diff
    }

    fn set_viewport_top_at(&mut self, viewport_top: Id<HistoryRow>, pos: usize) {
        let prev = self.rows[viewport_top].prev;
        if let Some(row) = self.row_offset(prev, self.max_height as _) {
            self.set_viewport_bottom_at(row, pos + self.max_height - 1);
        } else {
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    fn compute_item_row(&self, item: Id<HistoryItem>, base: Option<(Id<HistoryItem>, usize)>) -> usize {
        if item == self.first_item().unwrap() {
            return 0
        } else if item == self.last_item().unwrap() {
            return self.num_rows() - self.item[self.last_item().unwrap()].num_rows;
        }

        let (base_item, base_item_row) = base.unwrap_or_else(|| (self.first_item().unwrap(), 0));
        if base_item == item {
            return base_item_row;
        }

        if self.less_than(base_item, item) {
            base_item_row + self.item_row_diff(base_item, item)
        } else {
            base_item_row - self.item_row_diff(item, base_item)
        }
    }

    /// Attempts to set the viewport region based on last row. `pos` is the
    /// absolute row index of `viewport.bottom` (0-based from `first_row()`).
    fn set_viewport_bottom_at(&mut self, new_bottom: Id<HistoryRow>, new_row: usize) {
        if new_bottom == self.head { return; }

        if let Some(new_top) = self.row_offset(new_bottom, -(self.max_height as isize - 1)) {
            let base = self.bottom_item()
                .map(|item| (item, self.viewport.bottom_item_row));

            self.viewport.top = new_top;
            self.viewport.top_row = new_row - (self.max_height - 1);
            self.viewport.bottom = new_bottom;
            self.viewport.bottom_row = new_row;

            self.viewport.bottom_item_row = self.compute_item_row(self.rows[new_bottom].item, base);
        } else if new_bottom == self.last_row() {
            // Viewport covers full history
            self.viewport.top = self.first_row();
            self.viewport.top_row = 0;
            self.viewport.bottom = self.last_row();
            self.viewport.bottom_row = self.num_rows() - 1;
            self.viewport.bottom_item_row = self.item_row_diff(
                self.rows[self.first_row()].item,
                self.rows[self.last_row()].item,
            );
        } else {
            // Clip to top of history
            self.set_viewport_top_at(self.first_row(), 0);
        }
    }

    /// Slightly inefficient helper for tests
    #[cfg(test)]
    fn set_viewport_top(&mut self, viewport_top: Id<HistoryRow>) {
        let pos = self.row_diff(self.first_row(), viewport_top) as usize;
        self.set_viewport_top_at(viewport_top, pos);
    }

    pub fn max_height(&self) -> usize {
        self.max_height
    }

    /// Absolute row index of the first visible row (0-based from `first_row()`).
    pub fn viewport_top_pos(&self) -> usize {
        self.viewport.top_row
    }

    /// Absolute row index of the last visible row (0-based from `first_row()`).
    pub fn viewport_bottom_pos(&self) -> usize {
        self.viewport.bottom_row
    }

    /// Updates the maximum viewport size, preserving the viewport bottom.
    pub fn set_max_height(&mut self, max_height: usize) {
        if max_height == 0 {
            return;
        }
        self.max_height = max_height;
        self.set_viewport_bottom_at(self.viewport.bottom, self.viewport.bottom_row);
    }

    /// Updates the wrapping width, re-rendering all markdown items. The
    /// viewport continues to follow the newest messages.
    pub fn set_width(&mut self, width: usize) {
        if width == self.width {
            return;
        }
        self.width = width;

        let saved: Vec<HistoryItem> = self.item.drain().collect();

        self.item.clear();
        self.rows.clear();
        self.by_item_id.clear();

        let head = self.rows.insert(HistoryRow {
            item: Id::null(),
            preformatted: StyledString::new(self.theme.base_style(), 0),
            prev: Id::null(),
            next: Id::null(),
        });
        self.rows[head].prev = head;
        self.rows[head].next = head;
        self.head = head;
        self.viewport.top = head;
        self.viewport.bottom = head;
        self.viewport.top_row = 0;
        self.viewport.bottom_row = 0;
        self.viewport.bottom_item_row = 0;

        for saved in saved {
            let prev = self.last_row();
            let id = HistoryItem::create(
                self.theme,
                &mut self.item,
                &mut self.rows,
                prev,
                width,
                saved.content,
                saved.item_id,
            );
            self.item[id].seqno = saved.seqno;
            self.item[id].seq_offset = saved.seq_offset;
            if let Some(item_id) = saved.item_id {
                self.by_item_id.insert(item_id, id);
            }
        }
        self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
    }

    fn add_content(&mut self, content: Box<dyn RenderItem>) {
        self.add_item(content, None, None)
    }

    // TODO: decouple tool calls from tool outputs so they are sorted in
    // completion order
    fn find_insertion_point(&self, seqno: i64) -> Id<HistoryRow> {
        // Insert after first row with smaller seqno (or at beginning)
        let mut cur = self.last_row();
        while cur != self.head {
            let item = &self.item[self.rows[cur].item];
            if item.seqno < seqno {
                return cur;
            }
            cur = self.rows[item.first_row].prev;
        }
        self.head
    }

    fn next_sort_key(&self) -> (i64, u32) {
        if let Some(last) = self.last_item() {
            let last = &self.item[last];
            (last.seqno, last.seq_offset + 1)
        } else {
            (0, 0)
        }
    }

    fn add_item(
        &mut self,
        content: Box<dyn RenderItem>,
        item_id: Option<i32>,
        seqno: Option<i64>,
    ) {
        let prev = if let Some(seqno) = seqno {
            self.find_insertion_point(seqno)
        } else {
            self.last_row()
        };

        let (seqno, seq_offset) = match seqno {
            Some(seqno) => (seqno, 0),
            None => self.next_sort_key(),
        };

        // Add content
        let created_id = HistoryItem::create(
            self.theme,
            &mut self.item,
            &mut self.rows,
            prev,
            self.width,
            content,
            item_id,
        );
        let item = &mut self.item[created_id];
        item.seqno = seqno;
        item.seq_offset = seq_offset;
        if let Some(id) = item_id {
            self.by_item_id.insert(id, created_id);
        }

        // Update viewport
        let item = &self.item[created_id];
        if let Some(bottom_item) = self.bottom_item() {
            if item.sort_key() < self.item[bottom_item].sort_key() {
                self.viewport.top_row += item.num_rows;
                self.viewport.bottom_row += item.num_rows;
                self.viewport.bottom_item_row += item.num_rows;
            }
        } else {
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    /// Creates (or updates) an item
    fn on_item_created(&mut self, item: &Item) {
        if self.by_item_id.contains_key(&item.id) {
            self.on_item_updated(item);
        } else if let Some(content) = get_item_content(&self.tools, item) {
            self.add_item(content, Some(item.id), Some(item.seqno));
        }
    }

    fn update_item(&mut self, item_id: Id<HistoryItem>, content: Box<dyn RenderItem>) {
        // Update content
        let old_resume_point = self.item[item_id].resume_point;
        let old_num_rows = self.item[item_id].num_rows;
        self.item[item_id].update(
            &self.theme,
            &mut self.rows,
            self.head,
            self.width,
            content,
        );
        let new_num_rows = self.item[item_id].num_rows;

        // Update viewport
        let bottom_item = self.bottom_item().unwrap();
        match self.item[item_id].sort_key().cmp(&self.item[bottom_item].sort_key()) {
            std::cmp::Ordering::Less => {},
            std::cmp::Ordering::Equal => {
                let offset = self.viewport.bottom_row - self.viewport.bottom_item_row;
                if old_resume_point.row <= offset {
                    let offset = offset.min(new_num_rows); // Handle shrinking
                    let last_row = self.item[item_id].last_row;
                    let diff = offset as isize - new_num_rows as isize + 1;
                    // XXX: Maybe walk from first row if it's shorter
                    self.viewport.bottom = self.row_offset(last_row, diff).unwrap();
                }
            },
            std::cmp::Ordering::Greater => {
                let delta = new_num_rows - old_num_rows;
                self.viewport.bottom_row += delta;
                self.viewport.bottom_item_row += delta;
            },
        }
    }

    /// Does an incremental rerender for an updated item.
    fn on_item_updated(&mut self, item: &Item) {
        let Some(&item_id) = self.by_item_id.get(&item.id) else {
            self.on_item_created(item);
            return;
        };
        if let Some(content) = get_item_content(&self.tools, item) {
            self.update_item(item_id, content);
        }
    }

    fn scroll_up(&mut self, rows: usize) {
        if let Some(top) = self.row_offset(self.viewport.top, -(rows as isize)) {
            self.set_viewport_top_at(top, self.viewport.top_row - rows);
        } else {
            self.set_viewport_top_at(self.first_row(), 0);
        }
    }

    fn scroll_down(&mut self, rows: usize) {
        if let Some(bottom) = self.row_offset(self.viewport.bottom, rows as isize) {
            self.set_viewport_bottom_at(bottom, self.viewport.bottom_row + rows);
        } else {
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    fn do_update<'a>(&mut self, update: Update<'a>) {
        match update {
            Update::ItemCreated { item } => self.on_item_created(item),
            Update::ItemUpdated { item } => self.on_item_updated(item),
            Update::HelpMessage(content) => self.add_content(Box::new(HelpRenderItem::new(content))),
            Update::ErrorMessage(content) => self.add_content(Box::new(ErrorRenderItem::new(content))),
            Update::CommandPrompt(content) => {
                self.add_content(Box::new(CommandPromptRenderItem::new(content)))
            }
            Update::CommandOutput(content) => {
                self.add_content(Box::new(CommandOutputRenderItem::new(content)))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Update<'a> {
    ItemCreated { item: &'a Item },
    ItemUpdated { item: &'a Item },
    HelpMessage(&'a str),
    ErrorMessage(&'a str),
    CommandPrompt(&'a str),
    CommandOutput(&'a str),
}

impl Component for History {
    type Update<'a> = Update<'a>;
    type Event = ();

    fn draw(&self, canvas: Canvas) {
        let prev = self.rows[self.viewport.top].prev;
        for (i, (_, row)) in self.iter_range(prev, self.viewport.bottom).enumerate() {
            if i >= canvas.len() {
                break;
            }
            canvas[i].push_styled(&row.preformatted);
        }
    }

    fn set_width(&mut self, width: usize) {
        History::set_width(self, width);
    }

    fn set_height(&mut self, height: usize) {
        self.set_max_height(height);
    }

    fn set_focus(&mut self, _focused: bool) {}

    fn width(&self) -> usize {
        self.width
    }

    /// Number of actually visible rows.
    fn height(&self) -> usize {
        std::cmp::min(self.max_height, self.num_rows())
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        None
    }

    fn handle_input(&mut self, event: Event) -> Self::Event {
        let KeyEvent { code, modifiers, .. } = match event {
            Event::Key(key) => key,
            _ => return,
        };
        let alt = modifiers.contains(KeyModifiers::ALT);
        match (code, alt) {
            (KeyCode::Up, _) => self.scroll_up(1),
            (KeyCode::Down, _) => self.scroll_down(1),
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => self.scroll_up(self.height() / 2),
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => {
                self.scroll_down(self.height() / 2)
            }
            (KeyCode::Home, _) => self.set_viewport_top_at(self.first_row(), 0),
            (KeyCode::End, _) => self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1),
            _ => {}
        }
    }

    fn handle_update<'a>(&mut self, update: Self::Update<'a>) {
        self.do_update(update);
    }
}

fn by_item_id_json(map: &FnvHashMap<i32, Id<HistoryItem>>) -> Value {
    map.iter()
        .map(|(k, id)| (k.to_string(), (*id).to_json()))
        .collect()
}

impl DataQuery for History {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "num_rows": self.query("/num_rows")?,
                "items": self.query("/items")?,
                "width": self.query("/width")?,
                "max_height": self.query("/max_height")?,
                "head": self.query("/head")?,
                "viewport_top": self.query("/viewport_top")?,
                "viewport_top_pos": self.query("/viewport_top_pos")?,
                "viewport_bottom": self.query("/viewport_bottom")?,
                "viewport_bottom_pos": self.query("/viewport_bottom_pos")?,
                "by_item_id": self.query("/by_item_id")?,
            }))),
            "num_rows" => Ok(QueryField::Value(json!(self.num_rows()))),
            "items" => Ok(QueryField::Boxed(Box::new(HistoryItemsData {
                history: self,
            }))),
            "width" => Ok(QueryField::Value(json!(self.width))),
            "max_height" => Ok(QueryField::Value(json!(self.max_height))),
            "head" => Ok(QueryField::Value(self.head.to_json())),
            "viewport_top" => Ok(QueryField::Value(self.viewport.top.to_json())),
            "viewport_top_pos" => Ok(QueryField::Value(json!(self.viewport.top_row))),
            "viewport_bottom" => Ok(QueryField::Value(self.viewport.bottom.to_json())),
            "viewport_bottom_pos" => Ok(QueryField::Value(json!(self.viewport.bottom_row))),
            "by_item_id" => Ok(QueryField::Value(by_item_id_json(&self.by_item_id))),
            // "rows" not exposed
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct HistoryRowIter<'i> {
    rows: &'i Arena<HistoryRow>,
    prev: Id<HistoryRow>,
    last: Id<HistoryRow>,
}

impl<'i> Iterator for HistoryRowIter<'i> {
    type Item = (Id<HistoryRow>, &'i HistoryRow);

    fn next(&mut self) -> Option<Self::Item> {
        if self.prev == self.last {
            return None;
        }
        let id = self.rows[self.prev].next;
        self.prev = id;
        Some((id, &self.rows[id]))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        ((self.prev != self.last) as usize, None)
    }
}

impl<'i> DoubleEndedIterator for HistoryRowIter<'i> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.prev == self.last {
            return None;
        }
        let id = self.last;
        self.last = self.rows[self.last].prev;
        Some((id, &self.rows[id]))
    }
}

impl<'i> std::iter::FusedIterator for HistoryRowIter<'i> {}

/// Helper which implements DataQuery for `History.items`. It can fetch a
/// single item or all items at once by iteration.
#[derive(Debug)]
struct HistoryItemsData<'a> {
    history: &'a History,
}

impl<'h> DataQuery for HistoryItemsData<'h> {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        let items = self.history.item_ids();
        if field.is_empty() {
            let arr: Vec<Value> = items
                .iter()
                .map(|id| self.history.item[*id].query("/"))
                .collect::<Result<_, _>>()?;
            Ok(QueryField::Value(arr.into()))
        } else {
            let index: usize = field
                .parse()
                .map_err(|_| QueryError::InvalidField(field.to_string()))?;
            let id = items
                .get(index)
                .ok_or_else(|| QueryError::InvalidField(field.to_string()))?;
            Ok(QueryField::DataQuery(&self.history.item[*id]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fmt::Write;

    use crate::item::{ItemContent, ToolCallContent, ToolOutput};
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::ResponseRenderItem;
    use crate::ui::style::{Style, THEME_DARK};
    use crate::ui::style::testing::SetItalic;
    use serde_json::json;

    fn history(width: usize, max_height: usize) -> History {
        History::new(&crate::testing::ui_context(), width, max_height, &THEME_DARK)
    }

    fn item_contents(history: &History) -> Vec<String> {
        history
            .query("/items")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["content"]["value"].as_str().unwrap().to_string())
            .collect()
    }

    fn item_with_seqno(id: i32, seqno: i64) -> Item {
        Item {
            id,
            session_id: 1,
            turn_id: 1,
            response_id: None,
            provider_id: None,
            seqno,
            upstream_id: None,
            content: ItemContent::ResponseText(format!("message {seqno}")),
        }
    }

    fn message(num_lines: usize) -> String {
        let mut out = String::new();
        for i in 0..num_lines {
            let _ = write!(out, "- line {}\n", i);
        }
        out
    }

    fn update_item(history: &mut History, id: Id<HistoryItem>, delta: &str) {
        //let id = history.item.id_at(index);
        //let theme = history.theme;
        //let width = history.width;
        //let mut text = history.item[id].content.query("/value").unwrap()
        //    .as_str().expect("response value").to_string();
        //text.push_str(delta);
        //let content = Box::new(ResponseRenderItem::new(text));
        //history.item[id].update(theme, &mut history.rows, history.head, width, content);
    }

    fn render_draw(history: &History) -> String {
        let mut rows: Vec<StyledString> = (0..history.height())
            .map(|_| StyledString::new(history.theme.base_style(), history.width()))
            .collect();
        history.draw(&mut rows);
        render_canvas(&mut rows[..])
    }

    #[test]
    fn test_render_draw() {
        let theme = &THEME_DARK;

        let mut h = history(12, 5);
        assert_eq!(h.width(), 12);
        assert_eq!(h.height(), 0);
        assert_eq!(h.cursor(), None);
        assert_eq!(render_draw(&h), "");

        h.handle_update(Update::HelpMessage("hello"));
        assert_eq!(h.height(), 2);
        assert_eq!(h.num_rows(), 2);

        let help_style = Style::new(theme.text_subtle, theme.bg_base);
        let base_style = theme.base_style();
        let italic = SetItalic;

        assert_eq!(
            render_draw(&h),
            format!("{help_style}▐ {italic}hello     \n{base_style}            "),
        );

        let mut h = history(12, 2);
        h.handle_update(Update::HelpMessage("one\ntwo"));
        assert_eq!(h.num_rows(), 3);
        assert_eq!(h.height(), 2);

        assert_eq!(render_draw(&h), format!("{help_style}▐ {italic}two       \n{base_style}            "));

        h.scroll_up(1);
        assert_eq!(render_draw(&h), format!("{help_style}▐ {italic}one       \n{help_style}▐ {italic}two       "));

        h.set_width(14);
        assert_eq!(h.width(), 14);

        h.set_height(4);
        assert_eq!(h.height(), 3);
    }

    #[test]
    fn test_empty_history() {
        let history = history(20, 5);
        assert_eq!(history.num_rows(), 0);
        assert_eq!(history.item.len(), 0);
        assert_eq!(history.viewport.top, history.head);
        assert_eq!(history.viewport.bottom, history.head);
        assert_eq!(history.viewport_top_pos(), 0);
        assert_eq!(history.viewport_bottom_pos(), 0);
    }

    #[test]
    fn test_command_item_padding() {
        let mut h = history(20, 10);
        h.handle_update(Update::CommandPrompt("!echo"));
        assert_eq!(h.num_rows(), 1);

        h.handle_update(Update::CommandOutput("hi"));
        assert_eq!(h.num_rows(), 3);

        h.handle_update(Update::CommandOutput("line one\nline two"));
        assert_eq!(h.num_rows(), 6);
    }

    #[test]
    fn test_viewport_scroll() {
        // Scrolling
        let mut h = history(80, 2);
        for i in 0..10 {
            h.add_content(Box::new(ResponseRenderItem::new(format!("message {i}"))));
        }
        assert_eq!(h.num_rows(), 20);
        assert_eq!(h.viewport.bottom_row, 1);
        assert_eq!(h.viewport.bottom_item_row, 0);

        h.scroll_down(1);
        assert_eq!(h.viewport.bottom_row, 2);
        assert_eq!(h.viewport.bottom_item_row, 2);

        h.scroll_down(1);
        assert_eq!(h.viewport.bottom_row, 3);
        assert_eq!(h.viewport.bottom_item_row, 2);

        h.scroll_up(1);
        assert_eq!(h.viewport.bottom_row, 2);
        assert_eq!(h.viewport.bottom_item_row, 2);

        h.scroll_up(1000);
        assert_eq!(h.viewport.bottom_row, 1);
        assert_eq!(h.viewport.bottom_item_row, 0);

        h.scroll_down(1000);
        assert_eq!(h.viewport.bottom_row, 19);
        assert_eq!(h.viewport.bottom_item_row, 18);
    }

    #[test]
    fn test_viewport_insert() {
        let mut h = history(80, 4);

        // Initial insert
        add_item(&mut h, 1, 10, &message(9));
        assert_eq!(h.num_rows(), 10);
        assert_eq!(h.viewport.top_row, 6);
        assert_eq!(h.viewport.bottom_row, 9);
        assert_eq!(h.viewport.bottom_item_row, 0);

        // Insert below
        h.set_viewport_top(h.first_row());
        add_item(&mut h, 2, 20, &message(9));
        assert_eq!(h.num_rows(), 20);
        assert_eq!(h.viewport.top_row, 0);
        assert_eq!(h.viewport.bottom_row, 3);
        assert_eq!(h.viewport.bottom_item_row, 0);

        // Insert above
        add_item(&mut h, 3, 1, &message(9));
        assert_eq!(h.num_rows(), 30);
        assert_eq!(h.viewport.top_row, 10);
        assert_eq!(h.viewport.bottom_row, 13);
        assert_eq!(h.viewport.bottom_item_row, 10);
    }

    #[test]
    fn test_append() {
        //let full = "# Title\n\nfirst\n\nsecond\n\nthird";
        //
        //let mut incremental = history(20, 20);
        //incremental.add_content(Box::new(ResponseRenderItem::new("# Title")));
        //assert_eq!(incremental.num_rows(), 2);
        //update_item(&mut incremental, 0, "\n\nfirst");
        //assert_eq!(incremental.num_rows(), 4);
        //update_item(&mut incremental, 0, "\n\nsecond");
        //assert_eq!(incremental.num_rows(), 6);
        //update_item(&mut incremental, 0, "\n\nthird");
        //assert_eq!(incremental.num_rows(), 8);
        //
        //let mut whole = history(20, 20);
        //whole.add_content(Box::new(ResponseRenderItem::new(full)));
        //
        //assert_eq!(render_draw(&incremental), render_draw(&whole));
    }

    #[test]
    fn test_history_item_query() {
        let item = HistoryItem {
            content: Box::new(ResponseRenderItem::new("hello")),
            resume_point: ResumePoint { offset: 0, row: 0 },
            item_id: None,
            seqno: 7,
            seq_offset: 3,
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        };
        let expected = json!({
            "content": {"type": "response", "value": "hello"},
            "resume_point": item.resume_point.query("/").unwrap(),
            "item_id": null,
            "seqno": 7,
            "seq_offset": 3,
            "sort_key": [7, 3],
            "first_row": item.first_row.to_json(),
            "last_row": item.last_row.to_json(),
            "num_rows": 0,
        });
        assert_eq!(item.query("/").unwrap(), expected);
        assert_eq!(item.query("/content").unwrap(), json!({"type": "response", "value": "hello"}));
        assert_eq!(item.query("/content/type").unwrap(), json!("response"));
        assert_eq!(item.query("/content/value").unwrap(), json!("hello"));
        assert_eq!(
            item.query("/resume_point").unwrap(),
            item.resume_point.query("/").unwrap()
        );
        assert_eq!(item.query("/item_id").unwrap(), json!(null));
        assert_eq!(item.query("/seqno").unwrap(), json!(7));
        assert_eq!(item.query("/seq_offset").unwrap(), json!(3));
        assert_eq!(item.query("/sort_key").unwrap(), json!([7, 3]));
        assert_eq!(item.query("/first_row").unwrap(), item.first_row.to_json());
        assert_eq!(item.query("/last_row").unwrap(), item.last_row.to_json());
        assert_eq!(item.query("/num_rows").unwrap(), json!(0));
    }

    fn make_tool_call(
        id: i32,
        seqno: i64,
        call_id: &str,
        name: &str,
        args: Value,
        output: Option<Value>,
    ) -> Item {
        Item {
            id,
            session_id: 1,
            turn_id: 1,
            response_id: None,
            provider_id: None,
            seqno,
            upstream_id: None,
            content: ItemContent::ToolCall(ToolCallContent {
                tool_name: name.to_owned(),
                call_id: call_id.to_owned(),
                args,
                output: output.map(|value| ToolOutput::Completed { value }),
            }),
        }
    }

    #[test]
    fn test_tool_output() {
        let theme = &THEME_DARK;
        let style = Style::new(theme.text_base, theme.bg_prompt);
        let base_style = theme.base_style();
        let output = json!({ "stdout": "hello\n", "stderr": "", "return_code": 0 });

        let mut h = history(14, 10);
        let call = make_tool_call(1, 1, "call_1", "sh", json!({ "command": "echo hi" }), None);
        h.handle_update(Update::ItemCreated { item: &call });

        // Tool calls without output are ignored.
        assert_eq!(h.num_rows(), 0);
        assert_eq!(h.by_item_id.len(), 0);

        // When the output arrives, the item is inserted and rendered.
        let call = make_tool_call(
            1,
            1,
            "call_1",
            "sh",
            json!({ "command": "echo hi" }),
            Some(output.clone()),
        );
        h.handle_update(Update::ItemUpdated { item: &call });

        // Rendered with the prompt background and padding, plus the item's
        // own trailing padding row.
        assert_eq!(h.num_rows(), 5);
        assert_eq!(
            render_draw(&h),
            format!(
                "{style}              \n{style}  $ echo hi   \n{style}  hello       \n{style}              \n{base_style}              "
            )
        );
        // The item stores the parsed output; the renderer handles formatting.
        assert_eq!(
            h.query("/items/0/content").unwrap(),
            json!({"type": "sh", "cmd_line": "echo hi", "stdout": "hello\n"})
        );

        // Items created with output render immediately.
        let mut h = history(14, 10);
        let call = make_tool_call(
            2,
            2,
            "call_2",
            "sh",
            json!({ "command": "echo hi" }),
            Some(output),
        );
        h.handle_update(Update::ItemCreated { item: &call });
        assert_eq!(h.num_rows(), 5);
    }

    #[test]
    fn test_tool_call_missing_name() {
        let mut h = history(14, 10);
        let call = make_tool_call(
            1,
            1,
            "call_1",
            "",
            json!({ "command": "echo hi" }),
            Some(json!({ "stdout": "hello\n", "stderr": "", "return_code": 0 })),
        );
        h.handle_update(Update::ItemCreated { item: &call });

        // An empty tool name falls back to a placeholder.
        assert_eq!(
            h.query("/items/0/content").unwrap(),
            json!({"type": "help", "value": "Called '<missing name>'"})
        );
    }

    fn add_item(history: &mut History, id: i32, seqno: i64, message: &str) {
        let item = Item {
            id,
            session_id: 1,
            turn_id: 1,
            response_id: None,
            provider_id: None,
            seqno,
            upstream_id: None,
            content: ItemContent::ResponseText(message.to_owned()),
        };
        history.on_item_created(&item);
    }

    #[test]
    fn test_item_created_order() {
        let mut h = history(80, 10);

        h.handle_update(Update::HelpMessage("help"));

        let mut items: Vec<_> = [3, 1, 2]
            .into_iter()
            .enumerate()
            .map(|(i, seqno)| item_with_seqno(i as i32 + 1, seqno))
            .collect();
        for item in &items {
            h.do_update(Update::ItemCreated { item });
        }

        let seqnos: Vec<i64> = h
            .query("/items")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["seqno"].as_i64())
            .collect();
        assert_eq!(seqnos, [0, 1, 2, 3]);
        assert_eq!(item_contents(&h), ["help", "message 1", "message 2", "message 3"]);

        // No seqno: append at tail
        h.handle_update(Update::HelpMessage("help"));
        assert_eq!(
            item_contents(&h),
            ["help", "message 1", "message 2", "message 3", "help"]
        );

        // Append after items without seqno
        items.push(item_with_seqno(4, 4));
        h.handle_update(Update::ItemCreated { item: &items[3] });
        assert_eq!(
            item_contents(&h),
            ["help", "message 1", "message 2", "message 3", "help", "message 4"]
        );

        // Updated items are rerendered in place.
        let mut item = item_with_seqno(3, 2);
        item.content = ItemContent::ResponseText("updated".to_owned());
        h.handle_update(Update::ItemUpdated { item: &item });
        assert_eq!(
            item_contents(&h),
            ["help", "message 1", "updated", "message 3", "help", "message 4"]
        );
    }

    #[test]
    fn test_sort_key() {
        let mut h = history(80, 10);

        h.handle_update(Update::HelpMessage("one"));
        assert_eq!(h.query("/items/0/sort_key").unwrap(), json!([0, 0]));

        h.handle_update(Update::HelpMessage("two"));
        assert_eq!(h.query("/items/0/sort_key").unwrap(), json!([0, 0]));
        assert_eq!(h.query("/items/1/sort_key").unwrap(), json!([0, 1]));

        let item = item_with_seqno(1, 5);
        h.handle_update(Update::ItemCreated { item: &item });
        assert_eq!(h.query("/items/2/sort_key").unwrap(), json!([5, 0]));

        h.handle_update(Update::HelpMessage("three"));
        assert_eq!(h.query("/items/3/sort_key").unwrap(), json!([5, 1]));

        assert_eq!(h.query("/items/3/seqno").unwrap(), json!(5));
        assert_eq!(h.query("/items/3/seq_offset").unwrap(), json!(1));
    }

    #[test]
    fn test_history_query() {
        let history = history(20, 5);
        let expected = json!({
            "num_rows": 0,
            "items": [],
            "width": 20,
            "max_height": 5,
            "head": history.head.to_json(),
            "viewport_top": history.viewport.top.to_json(),
            "viewport_top_pos": 0,
            "viewport_bottom": history.viewport.bottom.to_json(),
            "viewport_bottom_pos": 0,
            "by_item_id": {},
        });
        assert_eq!(history.query("/").unwrap(), expected);
        assert_eq!(history.query("/num_rows").unwrap(), json!(0));
        assert_eq!(history.query("/items").unwrap(), json!([]));
        assert_eq!(history.query("/width").unwrap(), json!(20));
        assert_eq!(history.query("/max_height").unwrap(), json!(5));
        assert_eq!(history.query("/head").unwrap(), history.head.to_json());
        assert_eq!(
            history.query("/viewport_top").unwrap(),
            history.viewport.top.to_json()
        );
        assert_eq!(history.query("/viewport_top_pos").unwrap(), json!(0));
        assert_eq!(
            history.query("/viewport_bottom").unwrap(),
            history.viewport.bottom.to_json()
        );
        assert_eq!(history.query("/viewport_bottom_pos").unwrap(), json!(0));
        assert_eq!(history.query("/by_item_id").unwrap(), json!({}));
    }
}
