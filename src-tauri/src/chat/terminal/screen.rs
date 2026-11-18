//! One ordered emulator per native PTY. Provider process ownership stays in the conversation.
use crate::lock::lock;
use prometeu_core::terminal::{TerminalControl, TerminalSize};
use prometeu_process::terminal_state::{
    AppliedSnapshot, Client, Error, Key, Size, Snapshot, Stream,
};
use std::{process::Command, sync::Mutex, time::Duration};

pub(super) struct Screen(Mutex<State>);

enum State {
    Live {
        client: Client,
        stream: Stream,
        chunk: u64,
    },
    Frozen {
        chunk: u64,
        state: Box<Snapshot>,
    },
    Failed,
}

impl Screen {
    pub fn connected(&self) -> bool {
        matches!(*lock(&self.0), State::Live { .. })
    }

    pub fn launch(session: &str, generation: &str, size: TerminalSize) -> Result<Self, Error> {
        Self::with_command(command()?, session, generation, size)
    }

    fn with_command(
        command: Command,
        session: &str,
        generation: &str,
        size: TerminalSize,
    ) -> Result<Self, Error> {
        let mut client = Client::launch(command, Duration::from_secs(5))?;
        let stream = Stream::create(
            &mut client,
            Key {
                session: session.into(),
                generation: generation.into(),
            },
            Size {
                cols: size.cols,
                rows: size.rows,
            },
            1000,
        )?;
        Ok(Self(Mutex::new(State::Live {
            client,
            stream,
            chunk: 0,
        })))
    }

    pub fn write(&self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
        let mut state = lock(&self.0);
        let State::Live {
            client,
            stream,
            chunk,
        } = &mut *state
        else {
            return Err(Error::Closed);
        };
        let next = chunk.checked_add(1).ok_or(Error::Limit)?;
        match stream.write_chunk(client, next, bytes) {
            Ok(applied) => {
                *chunk = applied.chunk_seq;
                Ok(applied.replies)
            }
            Err(error) => {
                *state = State::Failed;
                Err(error)
            }
        }
    }

    /// Emulator resize and native resize share the same serialization boundary as output and
    /// snapshots. A failed second half invalidates the screen instead of advertising false size.
    pub fn resize(&self, control: &dyn TerminalControl, size: TerminalSize) -> Result<(), Error> {
        if !(2..=500).contains(&size.cols) || !(1..=300).contains(&size.rows) {
            return Err(Error::Limit);
        }
        let mut state = lock(&self.0);
        let State::Live { client, stream, .. } = &mut *state else {
            return Err(Error::Closed);
        };
        stream.resize(
            client,
            Size {
                cols: size.cols,
                rows: size.rows,
            },
        )?;
        if control.resize(size).is_err() {
            *state = State::Failed;
            return Err(Error::Io);
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<AppliedSnapshot, Error> {
        match &mut *lock(&self.0) {
            State::Live { client, stream, .. } => stream.snapshot(client),
            State::Frozen { chunk, state } => Ok(AppliedSnapshot {
                chunk_seq: *chunk,
                state: state.as_ref().clone(),
            }),
            State::Failed => Err(Error::Closed),
        }
    }

    /// An exited provider keeps its last screen without retaining a Node process per closed tab.
    pub fn freeze(&self) -> Result<(), Error> {
        let mut state = lock(&self.0);
        let old = std::mem::replace(&mut *state, State::Failed);
        match old {
            State::Live {
                mut client,
                mut stream,
                ..
            } => {
                let snapshot = stream.snapshot(&mut client)?;
                *state = State::Frozen {
                    chunk: snapshot.chunk_seq,
                    state: Box::new(snapshot.state),
                };
                let _ = stream.dispose(&mut client);
                client.shutdown()
            }
            saved @ State::Frozen { .. } => {
                *state = saved;
                Ok(())
            }
            State::Failed => Err(Error::Closed),
        }
    }
}

fn command() -> Result<Command, Error> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("PROMETEU_TERMINAL_STATE_HELPER") {
        return Ok(Command::new(path));
    }
    let executable = std::env::current_exe().map_err(|_| Error::Unavailable)?;
    let directory = executable.parent().ok_or(Error::Unavailable)?;
    Ok(Command::new(directory.join("prometeu-terminal-state")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Control(Mutex<Vec<TerminalSize>>);
    impl TerminalControl for Control {
        fn resize(&self, size: TerminalSize) -> Result<(), String> {
            lock(&self.0).push(size);
            Ok(())
        }
        fn running(&self) -> bool {
            true
        }
        fn system_id(&self) -> u32 {
            0
        }
        fn close(&self) {
            panic!("screen never owns the provider lifecycle");
        }
    }

    #[test]
    #[ignore = "requires PROMETEU_TERMINAL_STATE_HELPER pointing to the built SEA helper"]
    fn native_screen_tracks_chunks_replies_resize_and_freezes_without_a_helper() {
        let command = Command::new(
            std::env::var_os("PROMETEU_TERMINAL_STATE_HELPER").expect("built SEA helper"),
        );
        let screen = Screen::with_command(
            command,
            "screen-test",
            "generation",
            TerminalSize { cols: 20, rows: 4 },
        )
        .unwrap();
        assert!(screen.connected());
        let replies = screen
            .write(b"\x1b[?1049h\x1b[?2004h\x1b[2;3Hnative\x1b[6n")
            .unwrap();
        assert_eq!(replies, vec![b"\x1b[2;9R".to_vec()]);
        screen.write(&[0xf0, 0x9f]).unwrap();
        let pending = screen.snapshot().unwrap();
        assert_eq!(pending.chunk_seq, 2);
        assert_eq!(pending.state.pending_utf8, "8J8=");
        assert!(pending.state.modes.bracketed_paste_mode);
        assert_eq!(
            pending.state.active,
            prometeu_process::terminal_state::protocol::BufferKind::Alternate
        );
        let control = Control::default();
        assert_eq!(
            screen.resize(&control, TerminalSize { cols: 501, rows: 4 }),
            Err(Error::Limit)
        );
        assert!(lock(&control.0).is_empty());
        screen
            .resize(&control, TerminalSize { cols: 30, rows: 6 })
            .unwrap();
        assert_eq!(*lock(&control.0), [TerminalSize { cols: 30, rows: 6 }]);
        screen.write(&[0x98, 0x80]).unwrap();
        screen.write(b"").unwrap();
        let before = screen.snapshot().unwrap();
        assert_eq!(before.chunk_seq, 4);
        assert_eq!(before.state.cols, 30);
        assert!(before.state.pending_utf8.is_empty());
        screen.freeze().unwrap();
        assert!(!screen.connected());
        assert_eq!(screen.snapshot().unwrap().state, before.state);
        assert_eq!(screen.snapshot().unwrap().chunk_seq, before.chunk_seq);
        screen.freeze().unwrap();
        assert_eq!(screen.write(b"late"), Err(Error::Closed));
    }
}
