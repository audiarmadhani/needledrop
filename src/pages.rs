//! The middle pane: a stack of pages, newest on top. Opening something from
//! a page (an album on an artist page, say) pushes; `esc` goes back; picking
//! from the library or searching starts a new stack.

use ratatui::widgets::TableState;

use crate::model::{Item, Shelf, Track};

pub enum Content {
    /// A playlist, an album: songs in order.
    Tracks { tracks: Vec<Track>, state: TableState },
    /// Home, an artist, search results: shelves of cards.
    Shelves(Shelves),
}

/// Shelves of cards with a two-dimensional cursor: which shelf, and which
/// item along it. Each shelf remembers its own position, so moving up and
/// back down lands on the same card.
pub struct Shelves {
    pub shelves: Vec<Shelf>,
    /// The selected shelf.
    pub shelf: usize,
    /// The selected item in each shelf.
    pub columns: Vec<usize>,
    /// First shelf on screen; the renderer keeps the selection in view.
    pub scroll: usize,
}

impl Shelves {
    fn new(shelves: Vec<Shelf>) -> Self {
        let columns = vec![0; shelves.len()];
        Self {
            shelves,
            shelf: 0,
            columns,
            scroll: 0,
        }
    }

    /// The selected item in shelf `shelf`.
    pub fn column(&self, shelf: usize) -> usize {
        self.columns.get(shelf).copied().unwrap_or(0)
    }

    pub fn selected(&self) -> Option<&Item> {
        self.shelves.get(self.shelf)?.items.get(self.column(self.shelf))
    }

    /// Up and down between shelves; `isize::MIN` / `MAX` jump to the ends.
    fn move_shelf(&mut self, delta: isize) {
        if !self.shelves.is_empty() {
            self.shelf = self.shelf.saturating_add_signed(delta).min(self.shelves.len() - 1);
        }
    }

    /// Left and right along the selected shelf.
    fn move_across(&mut self, delta: isize) {
        let Some(len) = self.shelves.get(self.shelf).map(|s| s.items.len()) else {
            return;
        };
        if let (Some(column), true) = (self.columns.get_mut(self.shelf), len > 0) {
            *column = column.saturating_add_signed(delta).min(len - 1);
        }
    }
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
            Content::Shelves(Shelves::new(vec![]))
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
                self.subtitle = subtitle;
                self.content = Content::Shelves(Shelves::new(shelves));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        match &self.content {
            Content::Tracks { tracks, .. } => tracks.is_empty(),
            Content::Shelves(shelves) => shelves.shelves.is_empty(),
        }
    }

    pub fn is_shelves(&self) -> bool {
        matches!(self.content, Content::Shelves(_))
    }

    /// Up and down: songs in a list, shelves on a shelves page.
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
            Content::Shelves(shelves) => shelves.move_shelf(delta),
        }
    }

    /// Left and right along a shelf (no-op on a song list).
    pub fn move_across(&mut self, delta: isize) {
        if let Content::Shelves(shelves) = &mut self.content {
            shelves.move_across(delta);
        }
    }

    pub fn selected_item(&self) -> Option<&Item> {
        match &self.content {
            Content::Shelves(shelves) => shelves.selected(),
            Content::Tracks { .. } => None,
        }
    }

    /// The selected song, on either kind of page.
    pub fn selected_track(&self) -> Option<&Track> {
        match &self.content {
            Content::Tracks { tracks, state } => tracks.get(state.selected()?),
            Content::Shelves(shelves) => match shelves.selected()? {
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
            Content::Shelves(shelves) => {
                let items = &shelves.shelves.get(shelves.shelf)?.items;
                let item = shelves.column(shelves.shelf);
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
            thumbnail: None,
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
                    items: vec![album("y"), album("z")],
                },
            ],
            subtitle: None,
        });
        page
    }

    #[test]
    fn moves_across_and_between_shelves_remembering_each_position() {
        let mut page = shelves_page();
        assert_eq!(page.selected_item(), Some(&song("a")));
        page.move_across(2);
        assert_eq!(page.selected_item(), Some(&song("b")));
        page.move_across(5);
        assert_eq!(page.selected_item(), Some(&song("b")), "stops at the end of the shelf");
        page.move_selection(1);
        assert_eq!(
            page.selected_item(),
            Some(&album("y")),
            "the next shelf starts where it was left"
        );
        page.move_across(1);
        page.move_selection(-1);
        assert_eq!(page.selected_item(), Some(&song("b")), "back up to where we were");
        page.move_selection(isize::MAX);
        assert_eq!(page.selected_item(), Some(&album("z")));
    }

    #[test]
    fn playing_from_a_shelf_queues_its_songs() {
        let mut page = shelves_page();
        page.move_across(2);
        let (queue, index) = page.queue_from_selection().unwrap();
        let ids: Vec<_> = queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!((ids, index), (vec!["a", "b"], 1), "albums are left out; it starts on b");
        page.move_across(-1);
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
