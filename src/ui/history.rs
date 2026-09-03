//! Scrolling chat history. Data is constructed as a linked list of rows for
//! fast viewport scrolling and rendering.

use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use fnv::FnvHashMap;
use log::error;
use serde_json::{Value, json};

use crate::arena::{Arena, Id};
use crate::error::AnyResult;
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::session::{DbItem, ItemType};
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
    seqno: Option<i64>,
    first_row: Id<HistoryRow>,
    last_row: Id<HistoryRow>,
    num_rows: usize,
}

impl HistoryItem {
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
            seqno: None,
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
                "first_row": self.query("/first_row")?,
                "last_row": self.query("/last_row")?,
                "num_rows": self.query("/num_rows")?,
            }))),
            "content" => Ok(QueryField::DataQuery(&*self.content)),
            "resume_point" => Ok(QueryField::DataQuery(&self.resume_point)),
            "item_id" => Ok(QueryField::Value(json!(self.item_id))),
            "seqno" => Ok(QueryField::Value(json!(self.seqno))),
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
    viewport_top: Id<HistoryRow>,
    /// Absolute row index of `viewport_top`
    viewport_top_pos: usize,
    viewport_bottom: Id<HistoryRow>,
    /// Absolute row index of `viewport_bottom`
    viewport_bottom_pos: usize,
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
            viewport_top: head,
            viewport_bottom: head,
            viewport_top_pos: 0,
            viewport_bottom_pos: 0,
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

    /// Attempts to set the viewport region based on first row. `pos` is the
    /// absolute row index of `viewport_top` (0-based from `first_row()`).
    fn set_viewport_top_at(&mut self, viewport_top: Id<HistoryRow>, pos: usize) {
        let prev = self.rows[viewport_top].prev;
        self.viewport_top = viewport_top;
        self.viewport_top_pos = pos;
        if let Some(row) = self.row_offset(prev, self.max_height as _) {
            self.viewport_bottom = row;
            self.viewport_bottom_pos = pos + self.max_height - 1;
        } else if viewport_top == self.first_row() {
            self.viewport_bottom = self.last_row();
            self.viewport_bottom_pos = self.num_rows() - 1;
        } else {
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    /// Attempts to set the viewport region based on last row. `pos` is the
    /// absolute row index of `viewport_bottom` (0-based from `first_row()`).
    fn set_viewport_bottom_at(&mut self, viewport_bottom: Id<HistoryRow>, pos: usize) {
        self.viewport_bottom = viewport_bottom;
        self.viewport_bottom_pos = pos;
        if let Some(row) = self.row_offset(self.viewport_bottom, -(self.max_height as isize - 1)) {
            self.viewport_top = row;
            self.viewport_top_pos = pos - (self.max_height - 1);
        } else {
            // Viewport covers entire text
            self.viewport_top = self.first_row();
            self.viewport_top_pos = 0;
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
        self.viewport_top_pos
    }

    /// Absolute row index of the last visible row (0-based from `first_row()`).
    pub fn viewport_bottom_pos(&self) -> usize {
        self.viewport_bottom_pos
    }

    /// Updates the maximum viewport size, preserving the viewport bottom.
    pub fn set_max_height(&mut self, max_height: usize) {
        if max_height == 0 {
            return;
        }
        self.max_height = max_height;
        self.set_viewport_bottom_at(self.viewport_bottom, self.viewport_bottom_pos);
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
        self.viewport_top = head;
        self.viewport_bottom = head;
        self.viewport_top_pos = 0;
        self.viewport_bottom_pos = 0;

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
            if let Some(item_id) = saved.item_id {
                self.by_item_id.insert(item_id, id);
            }
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    fn add_content(&mut self, content: Box<dyn RenderItem>) {
        self.add_item(content, None, None)
    }

    fn find_insertion_point(&self, seqno: i64) -> Id<HistoryRow> {
        // Insert right before the last item with larger seqno, or at the end
        let mut cur = self.last_row();
        let mut result = cur;
        while cur != self.head {
            let item = &self.item[self.rows[cur].item];
            cur = self.rows[item.first_row].prev;
            if let Some(seqno2) = item.seqno {
                if seqno2 < seqno {
                    return result;
                } else {
                    result = cur;
                }
            }
        }
        result
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

        let created_id = HistoryItem::create(
            self.theme,
            &mut self.item,
            &mut self.rows,
            prev,
            self.width,
            content,
            item_id,
        );
        self.item[created_id].seqno = seqno;
        if let Some(id) = item_id {
            self.by_item_id.insert(id, created_id);
        }

        // Update viewport
        // FIXME: Scrolling
        self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
    }

    /// Creates (or updates) an item
    fn on_item_created(&mut self, item: &DbItem) -> AnyResult<()> {
        if item.ty()? == ItemType::ToolCall {
            if item.tool_output.is_none() {
                return Ok(());
            }
            // TODO: insert tool calls in completion order, not seqno order
        }

        if self.by_item_id.contains_key(&item.id) {
            self.on_item_updated(item)?;
        } else if let Some(content) = get_item_content(&self.tools, item)? {
            self.add_item(content, Some(item.id), Some(item.seqno));
        }

        Ok(())
    }

    /// Does an incremental rerender for an updated item. Updates nothing on
    /// error.
    fn on_item_updated(&mut self, item: &DbItem) -> AnyResult<()> {
        if let Some(&item_id) = self.by_item_id.get(&item.id) {
            let Some(content) = get_item_content(&self.tools, item)?
                else { return Ok(()) };
            self.item[item_id].update(
                &self.theme,
                &mut self.rows,
                self.head,
                self.width,
                content,
            );
            // FIXME: Should only track output if already at the bottom!
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        } else {
            self.on_item_created(item)?;
        }
        Ok(())
    }

    fn scroll_up(&mut self, rows: usize) {
        let (top, pos) = match self.row_offset(self.viewport_top, -(rows as isize)) {
            Some(row) => (row, self.viewport_top_pos - rows),
            None => (self.first_row(), 0),
        };
        self.set_viewport_top_at(top, pos);
    }

    fn scroll_down(&mut self, rows: usize) {
        if let Some(top) = self.row_offset(self.viewport_top, rows as isize) {
            self.set_viewport_top_at(top, self.viewport_top_pos + rows);
        } else {
            // Can't scroll any further; anchor to the bottom.
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    fn do_update<'a>(&mut self, update: Update<'a>) -> AnyResult<()> {
        match update {
            Update::ItemCreated { item } => self.on_item_created(item)?,
            Update::ItemUpdated { item } => self.on_item_updated(item)?,
            Update::HelpMessage(content) => self.add_content(Box::new(HelpRenderItem::new(content))),
            Update::ErrorMessage(content) => self.add_content(Box::new(ErrorRenderItem::new(content))),
            Update::CommandPrompt(content) => {
                self.add_content(Box::new(CommandPromptRenderItem::new(content)))
            }
            Update::CommandOutput(content) => {
                self.add_content(Box::new(CommandOutputRenderItem::new(content)))
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Update<'a> {
    ItemCreated { item: &'a DbItem },
    ItemUpdated { item: &'a DbItem },
    HelpMessage(&'a str),
    ErrorMessage(&'a str),
    CommandPrompt(&'a str),
    CommandOutput(&'a str),
}

impl Component for History {
    type Update<'a> = Update<'a>;
    type Event = ();

    fn draw(&self, canvas: Canvas) {
        let prev = self.rows[self.viewport_top].prev;
        for (i, (_, row)) in self.iter_range(prev, self.viewport_bottom).enumerate() {
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
        if let Err(e) = self.do_update(update) {
            error!("{}", e);
        }
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
            "viewport_top" => Ok(QueryField::Value(self.viewport_top.to_json())),
            "viewport_top_pos" => Ok(QueryField::Value(json!(self.viewport_top_pos))),
            "viewport_bottom" => Ok(QueryField::Value(self.viewport_bottom.to_json())),
            "viewport_bottom_pos" => Ok(QueryField::Value(json!(self.viewport_bottom_pos))),
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

    use crate::session::ItemType;
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::ResponseRenderItem;
    use crate::ui::style::{Style, THEME_DARK};
    use crate::ui::style::testing::SetItalic;
    use chrono::{DateTime, Utc};
    use serde_json::json;

    fn history(width: usize, max_height: usize) -> History {
        History::new(&crate::testing::ui_context(), width, max_height, &THEME_DARK)
    }

    fn update_item(history: &mut History, index: usize, delta: &str) {
        let id = history.item.id_at(index);
        let theme = history.theme;
        let width = history.width;
        let mut text = history.item[id].content.query("/value").unwrap()
            .as_str().expect("response value").to_string();
        text.push_str(delta);
        let content = Box::new(ResponseRenderItem::new(text));
        history.item[id].update(theme, &mut history.rows, history.head, width, content);
        history.set_viewport_bottom_at(history.last_row(), history.num_rows() - 1);
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
        assert_eq!(history.viewport_top, history.head);
        assert_eq!(history.viewport_bottom, history.head);
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
    fn test_scroll() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.add_content(Box::new(ResponseRenderItem::new(format!("message {i}"))));
        }
        assert_eq!(history.num_rows(), 20);

        // New items are anchored to the bottom.
        let last = history.last_row();
        let top = history.viewport_top;
        assert_eq!(history.viewport_bottom, last);
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);
        assert_ne!(top, history.first_row());

        history.scroll_up(1);
        assert_ne!(history.viewport_bottom, last);
        assert_ne!(history.viewport_top, top);
        assert_eq!(history.viewport_top_pos(), 15);
        assert_eq!(history.viewport_bottom_pos(), 18);

        history.scroll_down(1);
        assert_eq!(history.viewport_top, top);
        assert_eq!(history.viewport_bottom, last);
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);

        // Hit bottom
        history.scroll_down(1000);
        assert_eq!(history.viewport_bottom, last);
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);

        // Hit top
        history.scroll_up(1000);
        assert_eq!(history.viewport_top, history.first_row());
        assert_eq!(history.viewport_top_pos(), 0);
        assert_eq!(history.viewport_bottom_pos(), 3);
    }

    #[test]
    fn test_set_viewport_top_pos() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.add_content(Box::new(ResponseRenderItem::new(format!("message {i}"))));
        }

        let row = history.row_offset(history.first_row(), 5).unwrap();
        history.set_viewport_top(row);
        assert_eq!(history.viewport_top, row);
        assert_eq!(history.viewport_top_pos(), 5);
        assert_eq!(history.viewport_bottom_pos(), 8);
    }

    #[test]
    fn test_home_end() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.add_content(Box::new(ResponseRenderItem::new(format!("message {i}"))));
        }

        // Start at the bottom; scroll up so we're not at either extreme.
        history.scroll_up(5);
        assert_ne!(history.viewport_top, history.first_row());
        assert_ne!(history.viewport_bottom, history.last_row());
        assert_eq!(history.viewport_top_pos(), 11);
        assert_eq!(history.viewport_bottom_pos(), 14);

        // End scrolls the viewport to the last row.
        history.handle_input(Event::Key(KeyEvent::from(KeyCode::End)));
        assert_eq!(history.viewport_bottom, history.last_row());
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);

        // Home scrolls the viewport to the first row.
        history.handle_input(Event::Key(KeyEvent::from(KeyCode::Home)));
        assert_eq!(history.viewport_top, history.first_row());
        assert_eq!(history.viewport_top_pos(), 0);
        assert_eq!(history.viewport_bottom_pos(), 3);
    }

    #[test]
    fn test_append() {
        let full = "# Title\n\nfirst\n\nsecond\n\nthird";

        let mut incremental = history(20, 20);
        incremental.add_content(Box::new(ResponseRenderItem::new("# Title")));
        assert_eq!(incremental.num_rows(), 2);
        update_item(&mut incremental, 0, "\n\nfirst");
        assert_eq!(incremental.num_rows(), 4);
        update_item(&mut incremental, 0, "\n\nsecond");
        assert_eq!(incremental.num_rows(), 6);
        update_item(&mut incremental, 0, "\n\nthird");
        assert_eq!(incremental.num_rows(), 8);

        let mut whole = history(20, 20);
        whole.add_content(Box::new(ResponseRenderItem::new(full)));

        assert_eq!(render_draw(&incremental), render_draw(&whole));
    }

    #[test]
    fn test_history_item_query() {
        let item = HistoryItem {
            content: Box::new(ResponseRenderItem::new("hello")),
            resume_point: ResumePoint { offset: 0, row: 0 },
            item_id: None,
            seqno: Some(7),
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        };
        let expected = json!({
            "content": {"type": "response", "value": "hello"},
            "resume_point": item.resume_point.query("/").unwrap(),
            "item_id": null,
            "seqno": 7,
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
    ) -> DbItem {
        DbItem {
            id,
            session_id: 1,
            turn_id: 1,
            response_id: None,
            provider_id: None,
            ty: ItemType::ToolCall.to_string(),
            upstream_id: None,
            upstream_type: Some("function_call".into()),
            upstream_call_id: Some(call_id.into()),
            text: Some(name.into()),
            summary: None,
            encrypted_text: None,
            tool_args: Some(args.to_string()),
            raw_data: None,
            seqno,
            created_at: DateTime::<Utc>::UNIX_EPOCH.naive_utc(),
            updated_at: DateTime::<Utc>::UNIX_EPOCH.naive_utc(),
            tool_output: output.map(|v| v.to_string()),
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
        let mut call = call.clone();
        call.tool_output = Some(output.to_string());
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
        let mut call = make_tool_call(
            1,
            1,
            "call_1",
            "sh",
            json!({ "command": "echo hi" }),
            Some(json!({ "stdout": "hello\n", "stderr": "", "return_code": 0 })),
        );
        call.text = None;
        h.handle_update(Update::ItemCreated { item: &call });

        // Unparseable calls fall back to an unknown tool placeholder.
        assert_eq!(h.by_item_id.len(), 1);
        assert_eq!(
            h.query("/items/0/content").unwrap(),
            json!({"type": "help", "value": "Called '<missing name>'"})
        );

        // An empty name is treated the same way.
        let mut h = history(14, 10);
        call.text = Some(String::new());
        h.handle_update(Update::ItemCreated { item: &call });
        assert_eq!(
            h.query("/items/0/content").unwrap(),
            json!({"type": "help", "value": "Called '<missing name>'"})
        );
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

    fn item_with_seqno(id: i32, seqno: i64) -> DbItem {
        DbItem {
            id,
            session_id: 1,
            turn_id: 1,
            response_id: None,
            provider_id: None,
            ty: ItemType::ResponseText.to_string(),
            upstream_id: None,
            upstream_type: None,
            upstream_call_id: None,
            text: Some(format!("message {seqno}")),
            summary: None,
            encrypted_text: None,
            tool_args: None,
            raw_data: None,
            seqno,
            created_at: DateTime::<Utc>::UNIX_EPOCH.naive_utc(),
            updated_at: DateTime::<Utc>::UNIX_EPOCH.naive_utc(),
            tool_output: None,
        }
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
            h.do_update(Update::ItemCreated { item }).unwrap();
        }

        let seqnos: Vec<i64> = h
            .query("/items")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["seqno"].as_i64())
            .collect();
        assert_eq!(seqnos, [1, 2, 3]);
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
        item.text = Some("updated".into());
        h.handle_update(Update::ItemUpdated { item: &item });
        assert_eq!(
            item_contents(&h),
            ["help", "message 1", "updated", "message 3", "help", "message 4"]
        );
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
            "viewport_top": history.viewport_top.to_json(),
            "viewport_top_pos": 0,
            "viewport_bottom": history.viewport_bottom.to_json(),
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
            history.viewport_top.to_json()
        );
        assert_eq!(history.query("/viewport_top_pos").unwrap(), json!(0));
        assert_eq!(
            history.query("/viewport_bottom").unwrap(),
            history.viewport_bottom.to_json()
        );
        assert_eq!(history.query("/viewport_bottom_pos").unwrap(), json!(0));
        assert_eq!(history.query("/by_item_id").unwrap(), json!({}));
    }
}
