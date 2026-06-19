use crate::tui::state::{SessionState, TranscriptEntry};

pub struct TranscriptTurn<'a> {
    pub ordinal: usize,
    pub user_index: usize,
    pub end_message_index: usize,
    pub user_message: &'a TranscriptEntry,
    pub assistant_messages: Vec<(usize, &'a TranscriptEntry)>,
    pub live_text: Option<&'a str>,
    pub active: bool,
}

impl<'a> TranscriptTurn<'a> {
    pub fn last_assistant_index(&self) -> Option<usize> {
        self.assistant_messages.last().map(|(index, _)| *index)
    }

    pub fn primary_message_index(&self) -> usize {
        self.last_assistant_index().unwrap_or(self.user_index)
    }

    pub fn has_visible_assistant_content(&self) -> bool {
        !self.assistant_messages.is_empty() || self.live_text.is_some()
    }
}

pub fn build_transcript_turns(session: &SessionState) -> Vec<TranscriptTurn<'_>> {
    #[derive(Debug)]
    struct DraftTurn {
        ordinal: usize,
        user_index: usize,
        end_message_index: usize,
        assistant_indices: Vec<usize>,
    }

    let mut drafts = Vec::new();
    let mut current: Option<DraftTurn> = None;
    let mut ordinal = 0usize;

    for (index, entry) in session.transcript.iter().enumerate() {
        match entry.role {
            "user" => {
                if let Some(turn) = current.take() {
                    drafts.push(turn);
                }

                current = Some(DraftTurn {
                    ordinal,
                    user_index: index,
                    end_message_index: index,
                    assistant_indices: Vec::new(),
                });
                ordinal += 1;
            }
            "assistant" => {
                if let Some(turn) = current.as_mut() {
                    turn.assistant_indices.push(index);
                    turn.end_message_index = index;
                }
            }
            _ => {}
        }
    }

    if let Some(turn) = current.take() {
        drafts.push(turn);
    }

    let mut turns: Vec<TranscriptTurn<'_>> = drafts
        .into_iter()
        .filter_map(|draft| {
            let user_message = session.transcript.get(draft.user_index)?;
            Some(TranscriptTurn {
                ordinal: draft.ordinal,
                user_index: draft.user_index,
                end_message_index: draft.end_message_index,
                user_message,
                assistant_messages: draft
                    .assistant_indices
                    .into_iter()
                    .filter_map(|index| {
                        session.transcript.get(index).map(|entry| (index, entry))
                    })
                    .collect(),
                live_text: None,
                active: false,
            })
        })
        .collect();

    if let Some(last) = turns.last_mut() {
        if !session.streaming_response.is_empty() {
            last.live_text = Some(&session.streaming_response);
        }
        last.active = session.running;
    }

    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::SessionState;

    #[test]
    fn build_turns_empty_transcript() {
        let session = SessionState::new(50);
        let turns = build_transcript_turns(&session);
        assert!(turns.is_empty());
    }

    #[test]
    fn build_turns_single_user_message() {
        let mut session = SessionState::new(50);
        session.transcript.push(TranscriptEntry {
            role: "user",
            content: "Hello".to_string(),
        });
        let turns = build_transcript_turns(&session);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_message.content, "Hello");
    }

    #[test]
    fn build_turns_user_assistant_pair() {
        let mut session = SessionState::new(50);
        session.transcript.push(TranscriptEntry {
            role: "user",
            content: "Hello".to_string(),
        });
        session.transcript.push(TranscriptEntry {
            role: "assistant",
            content: "Hi there!".to_string(),
        });
        let turns = build_transcript_turns(&session);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].assistant_messages.len(), 1);
    }
}
