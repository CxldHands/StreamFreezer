use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Connected,
    Follow {
        event_id: String,
        user: String,
        avatar: String,
    },
}

#[cfg(test)]
mod tests {
    use super::ServerMessage;

    #[test]
    fn follow_message_uses_the_overlay_contract() {
        let message = ServerMessage::Follow {
            event_id: "synthetic-123".to_owned(),
            user: "TestFollower".to_owned(),
            avatar: "/assets/avatar.svg".to_owned(),
        };

        let json = serde_json::to_value(message).expect("message should serialize");
        assert_eq!(json["type"], "follow");
        assert_eq!(json["event_id"], "synthetic-123");
        assert_eq!(json["user"], "TestFollower");
        assert_eq!(json["avatar"], "/assets/avatar.svg");
    }

    #[test]
    fn connected_message_has_a_stable_type() {
        let json =
            serde_json::to_value(ServerMessage::Connected).expect("message should serialize");
        assert_eq!(json["type"], "connected");
    }
}
