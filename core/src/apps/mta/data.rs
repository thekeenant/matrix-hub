// Internal (post-processed) data model for the MTA app runtime state.
// These types are what the UI layer works with — no JSON coupling.

#[derive(Clone, Debug)]
pub struct Train {
    pub route: String,
    pub arrives_in_secs: u64,
    pub terminal_stop_id: String,
}

#[derive(Clone, Debug)]
pub struct Platform {
    pub direction: String,  // "N", "S", "E", "W"
    pub trains: Vec<Train>, // sorted by arrival time, closest first
}

#[derive(Clone, Debug)]
pub enum StationState {
    Loading,
    NoTrains,
    /// Fetching has failed repeatedly; we have no current data
    Unavailable,
    Live(Vec<Platform>),
}

impl StationState {
    /// The live platforms, or none if there's no live data.
    pub fn platforms(&self) -> &[Platform] {
        match self {
            Self::Live(platforms) => platforms,
            _ => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct StationData {
    pub route: String,
    pub state: StationState,
}
