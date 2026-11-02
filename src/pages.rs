//! The middle pane: a stack of pages, newest on top. Opening something from
//! a page (an album on an artist page, say) pushes; `esc` goes back; picking
//! from the library or searching starts a new stack.

use ratatui::widgets::{ListState, TableState};

use crate::model::{Item, Shelf, Track};

pub enum Content {
    /// A playlist, an album: songs in order.
    Tracks { tracks: Vec<Track>, state: TableState },
    /// Home, an artist, search results: titled shelves of items.
    Shelves {
        shelves: Vec<Shelf>,
        rows: Vec<Row>,
        state: ListState,
    },
}

/// A line in a shelves page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A shelf's title.
    Heading(usize),
    /// An item: shelf index, item index.
    Item(usize, usize),
}

pub struct Page {
    pub title: String,
    /// A line of detail, like an album's artist and year.
    pub subtitle: Option<String>,
    pub loading: bool,
    /// Which load fills this page, so a late response can't land on another.
    pub request: u64,
    pub content: Content,
}

/// What a finished load puts on its page.
pub enum Loaded {
    Tracks {
        tracks: Vec<Track>,
        subtitle: Option<String>,
    },
    Shelves {
        shelves: Vec<Shelf>,
        subtitle: Option<String>,
    },
}

impl Page {
    pub fn loading(title: String, request: u64, shelves: bool) -> Self {
        let content = if shelves {
            Content::Shelves {
                shelves: vec![],
                rows: vec![],
                state: ListState::default(),
            }
        } else {
            Content::Tracks {
                tracks: vec![],
                state: TableState::default(),
            }
        };
        Self {
            title,
            subtitle: None,
            loading: true,
            request,
            content,
        }
    }

    pub fn fill(&mut self, loaded: Loaded) {
        self.loading = false;
        match loaded {
            Loaded::Tracks { tracks, subtitle } => {
                let mut state = TableState::default();
                state.select((!tracks.is_empty()).then_some(0));
                self.subtitle = subtitle;
                self.content = Content::Tracks { tracks, state };
            }
            Loaded::Shelves { shelves, subtitle } => {
                let rows = rows(&shelves);
                let mut state = ListState::default();
                state.select(rows.iter().position(|row| matches!(row, Row::Item(..))));
                self.subtitle = subtitle;
                self.content = Content::Shelves { shelves, rows, state };
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.content {
            Content::Tracks { tracks, .. } => tracks.is_empty(),
            Content::Shelves { shelves, .. } => shelves.is_empty(),
        }
    }

    /// Moves the selection by `delta` items (headings are skipped);
    /// `isize::MIN` / `isize::MAX` jump to the first / last.
    pub fn move_selection(&mut self, delta: isize) {
        match &mut self.content {
            Content::Tracks { tracks, state } => {
                if !tracks.is_empty() {
                    let to = state
                        .selected()
                        .unwrap_or(0)
                        .saturating_add_signed(delta)
                        .min(tracks.len() - 1);
                    state.select(Some(to));
                }
            }
            Content::Shelves { rows, state, .. } => {
                let items: Vec<usize> = (0..rows.len()).filter(|&i| matches!(rows[i], Row::Item(..))).collect();
                if items.is_empty() {
                    return;
                }
                let at = state
                    .selected()
                    .and_then(|s| items.iter().position(|&i| i == s))
                    .unwrap_or(0);
                let to = at.saturating_add_signed(delta).min(items.len() - 1);
                state.select(Some(items[to]));
            }
        }
    }

    pub fn selected_item(&self) -> Option<&Item> {
        let Content::Shelves { shelves, rows, state } = &self.content else {
            return None;
        };
        match rows.get(state.selected()?)? {
            Row::Item(shelf, item) => shelves[*shelf].items.get(*item),
            Row::Heading(_) => None,
        }
    }

    /// The selected song, on either kind of page.
    pub fn selected_track(&self) -> Option<&Track> {
        match &self.content {
            Content::Tracks { tracks, state } => tracks.get(state.selected()?),
            Content::Shelves { .. } => match self.selected_item()? {
                Item::Song(track) => Some(track),
                _ => None,
            },
        }
    }

    /// The songs to queue when the selection is played, and where in them it
    /// is: the whole list, or the songs of the selected item's shelf.
    pub fn queue_from_selection(&self) -> Option<(Vec<Track>, usize)> {
        match &self.content {
            Content::Tracks { tracks, state } => Some((tracks.clone(), state.selected()?)),
            Content::Shelves { shelves, rows, state } => {
                let Row::Item(shelf, item) = *rows.get(state.selected()?)? else {
                    return None;
                };
                let items = &shelves[shelf].items;
                if !matches!(items.get(item)?, Item::Song(_)) {
                    return None;
                }
                let songs: Vec<(usize, &Track)> = items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, it)| match it {
                        Item::Song(track) => Some((i, track)),
                        _ => None,
                    })
                    .collect();
                let index = songs.iter().position(|(i, _)| *i == item)?;
                Some((songs.into_iter().map(|(_, track)| track.clone()).collect(), index))
            }
        }
    }
}

fn rows(shelves: &[Shelf]) -> Vec<Row> {
    shelves
        .iter()
        .enumerate()
        .flat_map(|(s, shelf)| {
            std::iter::once(Row::Heading(s)).chain((0..shelf.items.len()).map(move |i| Row::Item(s, i)))
        })
        .collect()
}

/// The page stack. Never empty once something is opened.
#[derive(Default)]
pub struct Nav {
    pages: Vec<Page>,
}

impl Nav {
    pub fn current(&self) -> Option<&Page> {
        self.pages.last()
    }

    pub fn current_mut(&mut self) -> Option<&mut Page> {
        self.pages.last_mut()
    }

    /// Whether `esc` has somewhere to go back to.
    pub fn can_go_back(&self) -> bool {
        self.pages.len() > 1
    }

    /// Opens a page on top of the current one.
    pub fn push(&mut self, page: Page) {
        self.pages.push(page);
    }

    /// Starts over with just this page (the library, a new search).
    pub fn reset(&mut self, page: Page) {
        self.pages = vec![page];
    }

    pub fn back(&mut self) {
        if self.can_go_back() {
            self.pages.pop();
        }
    }

    /// The page a load with this request id was for, if it's still open.
    pub fn waiting_for(&mut self, request: u64) -> Option<&mut Page> {
        self.pages.iter_mut().find(|page| page.request == request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Item {
        Item::Song(Track {
            video_id: id.into(),
            title: id.into(),
            ..Track::default()
        })
    }

    fn album(id: &str) -> Item {
        Item::Album {
            browse_id: id.into(),
            title: id.into(),
            subtitle: String::new(),
        }
    }

    fn shelves_page() -> Page {
        let mut page = Page::loading("Home".into(), 1, true);
        page.fill(Loaded::Shelves {
            shelves: vec![
                Shelf {
                    title: "Quick picks".into(),
                    items: vec![song("a"), album("x"), song("b")],
                },
                Shelf {
                    title: "Albums".into(),
                    items: vec![album("y")],
                },
            ],
            subtitle: None,
        });
        page
    }

    #[test]
    fn selection_skips_headings() {
        let mut page = shelves_page();
        assert_eq!(
            page.selected_item(),
            Some(&song("a")),
            "starts on the first item, not a heading"
        );
        page.move_selection(3);
        assert_eq!(page.selected_item(), Some(&album("y")), "crossed the Albums heading");
        page.move_selection(isize::MIN);
        assert_eq!(page.selected_item(), Some(&song("a")));
    }

    #[test]
    fn playing_from_a_shelf_queues_its_songs() {
        let mut page = shelves_page();
        page.move_selection(2);
        let (queue, index) = page.queue_from_selection().unwrap();
        let ids: Vec<_> = queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!((ids, index), (vec!["a", "b"], 1), "albums are left out; it starts on b");
        page.move_selection(-1);
        assert_eq!(page.queue_from_selection(), None, "an album isn't a song to queue");
    }

    #[test]
    fn stack_pushes_goes_back_and_matches_late_loads() {
        let mut nav = Nav::default();
        nav.reset(Page::loading("Home".into(), 1, true));
        nav.push(Page::loading("Album".into(), 2, false));
        assert!(nav.can_go_back());
        assert!(nav.waiting_for(1).is_some(), "a page underneath still accepts its load");
        nav.back();
        assert_eq!(nav.current().unwrap().title, "Home");
        nav.back();
        assert_eq!(nav.current().unwrap().title, "Home", "never pops the last page");
        assert!(nav.waiting_for(2).is_none());
    }
}
