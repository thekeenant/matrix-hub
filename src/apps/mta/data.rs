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
    Live(Vec<Platform>),
}

#[derive(Clone, Debug)]
pub struct StationData {
    pub route: String,
    pub state: StationState,
}
