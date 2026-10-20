//! Speech to text for the bottom panel (ticket 187): a Whisper model run on
//! this machine with candle, fed from the microphone. Nothing is sent
//! anywhere, and nothing is downloaded until the user chooses a model in
//! Preferences. And the other way (ticket 286): `say` reads text aloud with
//! the system's own speech.

pub mod audio;
pub mod dsp;
pub mod model;
pub mod say;
pub mod whisper;
