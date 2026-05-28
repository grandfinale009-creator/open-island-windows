use serde::{Deserialize, Serialize};

use crate::agent_event::AgentEvent;

/// Transport-agnostic NDJSON codec. Ported from `Sources/OpenIslandCore/BridgeTransport.swift`.
///
/// Protocol: one JSON object per line, terminated by `\n`.
/// The wire format is `BridgeEnvelope` serialized as JSON.

/// The top-level envelope for all bridge messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BridgeEnvelope {
    #[serde(rename = "hello")]
    Hello(BridgeHello),
    #[serde(rename = "event")]
    Event(AgentEvent),
    #[serde(rename = "command")]
    Command(BridgeCommand),
    #[serde(rename = "response")]
    Response(BridgeResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeHello {
    #[serde(default = "default_protocol_version")]
    pub protocol_version: u32,
    #[serde(default)]
    pub server_label: Option<String>,
}

fn default_protocol_version() -> u32 {
    1
}

/// Commands sent from hooks CLI to the bridge server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command")]
pub enum BridgeCommand {
    #[serde(rename = "registerClient")]
    RegisterClient,
    #[serde(rename = "processClaudeHook")]
    ProcessClaudeHook {
        #[serde(flatten)]
        payload: serde_json::Value,
    },
    #[serde(rename = "processCodexHook")]
    ProcessCodexHook {
        #[serde(flatten)]
        payload: serde_json::Value,
    },
    #[serde(rename = "processOpenCodeHook")]
    ProcessOpenCodeHook {
        #[serde(flatten)]
        payload: serde_json::Value,
    },
    #[serde(rename = "processCursorHook")]
    ProcessCursorHook {
        #[serde(flatten)]
        payload: serde_json::Value,
    },
    #[serde(rename = "processGeminiHook")]
    ProcessGeminiHook {
        #[serde(flatten)]
        payload: serde_json::Value,
    },
    #[serde(rename = "resolvePermission")]
    ResolvePermission {
        session_id: String,
        resolution: PermissionResolution,
    },
    #[serde(rename = "answerQuestion")]
    AnswerQuestion {
        session_id: String,
        response: QuestionResponse,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PermissionResolution {
    #[serde(rename = "allow")]
    Allow,
    #[serde(rename = "allowAlways")]
    AllowAlways,
    #[serde(rename = "deny")]
    Deny { #[serde(default)] reason: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionResponse {
    pub answers: Vec<Vec<String>>,
}

/// Responses sent from bridge server back to hooks CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BridgeResponse {
    #[serde(rename = "acknowledged")]
    Acknowledged,
    #[serde(rename = "directive")]
    Directive { payload: serde_json::Value },
}

/// Encode a single envelope as an NDJSON line (JSON + newline).
pub fn encode_line(envelope: &BridgeEnvelope) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec(envelope)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decode NDJSON buffer into envelopes. Returns parsed envelopes and remaining partial data.
pub fn decode_lines(buffer: &[u8]) -> (Vec<BridgeEnvelope>, Vec<u8>) {
    let mut envelopes = Vec::new();
    let mut remaining = buffer;

    while let Some(newline_pos) = remaining.iter().position(|&b| b == b'\n') {
        let line = &remaining[..newline_pos];
        remaining = &remaining[newline_pos + 1..];

        if line.is_empty() {
            continue;
        }

        match serde_json::from_slice::<BridgeEnvelope>(line) {
            Ok(env) => envelopes.push(env),
            Err(e) => {
                tracing::warn!("Failed to decode bridge envelope: {}", e);
            }
        }
    }

    (envelopes, remaining.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let envelope = BridgeEnvelope::Hello(BridgeHello {
            protocol_version: 1,
            server_label: Some("test".to_string()),
        });
        let encoded = encode_line(&envelope).unwrap();
        let (decoded, remaining) = decode_lines(&encoded);
        assert_eq!(remaining.len(), 0);
        assert_eq!(decoded.len(), 1);
    }

    #[test]
    fn test_decode_multiple_lines() {
        let mut data = Vec::new();
        let e1 = BridgeEnvelope::Hello(BridgeHello {
            protocol_version: 1,
            server_label: None,
        });
        let e2 = BridgeEnvelope::Hello(BridgeHello {
            protocol_version: 1,
            server_label: Some("second".to_string()),
        });
        data.extend_from_slice(&encode_line(&e1).unwrap());
        data.extend_from_slice(&encode_line(&e2).unwrap());

        let (decoded, remaining) = decode_lines(&data);
        assert_eq!(decoded.len(), 2);
        assert_eq!(remaining.len(), 0);
    }
}
