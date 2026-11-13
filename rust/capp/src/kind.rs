//! The kinds of operation a plan holds, and an operation's id as the execution engine names it.
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// An operation's kind: its `kind` in the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind { Facing, Roughing, Flats, SpotDrill, Holes, Counterbore, Slot, Notch, VChannel, WallFinish, EdgeChamfer, Thread }

impl OpKind {
    /// Every kind, in the order `rulebase/rules/sequence.json` cuts them.
    pub const ALL: [OpKind; 12] = [OpKind::Facing, OpKind::Roughing, OpKind::Flats, OpKind::SpotDrill, OpKind::Holes, OpKind::Counterbore,
        OpKind::Slot, OpKind::Notch, OpKind::VChannel, OpKind::WallFinish, OpKind::EdgeChamfer, OpKind::Thread];

    /// The kind as the plan writes it: `v_channel`.
    pub fn name(self) -> &'static str {
        match self {
            OpKind::Facing => "facing",
            OpKind::Roughing => "roughing",
            OpKind::Flats => "flats",
            OpKind::SpotDrill => "spot_drill",
            OpKind::Holes => "holes",
            OpKind::Counterbore => "counterbore",
            OpKind::Slot => "slot",
            OpKind::Notch => "notch",
            OpKind::VChannel => "v_channel",
            OpKind::WallFinish => "wall_finish",
            OpKind::EdgeChamfer => "edge_chamfer",
            OpKind::Thread => "thread",
        }
    }

    /// A short operation noun for reader-facing reasons; identifiers belong to the trace.
    pub(crate) fn noun(self) -> &'static str {
        match self {
            OpKind::Facing => "facing pass",
            OpKind::Roughing => "roughing pass",
            OpKind::Flats => "flat finishing pass",
            OpKind::SpotDrill => "spot drilling",
            OpKind::Holes => "holes",
            OpKind::Counterbore => "counterbore",
            OpKind::Slot => "slot",
            OpKind::Notch => "notch",
            OpKind::VChannel => "V-channel",
            OpKind::WallFinish => "wall finishing pass",
            OpKind::EdgeChamfer => "chamfer",
            OpKind::Thread => "thread",
        }
    }

    /// The process rule that plans an operation of this kind.
    pub(crate) fn process_rule(self) -> &'static str {
        match self {
            OpKind::Facing => "process.facing",
            OpKind::Roughing => "process.roughing",
            OpKind::Flats => "process.flats",
            OpKind::SpotDrill => "process.spot_drill",
            OpKind::Holes => "process.holes",
            OpKind::Counterbore => "process.counterbore",
            OpKind::Slot => "process.slot",
            OpKind::Notch => "process.notch",
            OpKind::VChannel => "process.v_channel",
            OpKind::WallFinish => "process.wall_finish",
            OpKind::EdgeChamfer => "process.edge_chamfer",
            OpKind::Thread => "process.thread",
        }
    }
}

impl FromStr for OpKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        OpKind::ALL.into_iter().find(|k| k.name() == s).ok_or_else(|| format!("unknown operation kind {s:?}"))
    }
}

impl fmt::Display for OpKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str(self.name()) }
}

/// An operation as the execution engine names it: `top-2-roughing`, or `top-3-spot_drill-2` for
/// the second depth group of a spot drill (the adapter cuts one spot operation per tip Z).
/// `number` counts the setup's operations from 1. Its JSON form is always the string, and
/// `Display` gives back exactly the text it was parsed from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OpId { pub setup: String, pub number: usize, pub kind: OpKind, pub part: Option<usize> }

/// A number written as `Display` writes it: digits, no sign, no leading zero.
fn number(s: &str) -> Option<usize> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))).then(|| s.parse().ok()).flatten()
}

impl FromStr for OpId {
    type Err = String;
    /// The only parser of an operation id.
    fn from_str(s: &str) -> Result<Self, String> {
        let not = || format!("{s:?} is not an operation id");
        let mut parts = s.splitn(3, '-');
        let (Some(setup), Some(n), Some(rest)) = (parts.next(), parts.next(), parts.next()) else { return Err(not()) };
        let (kind, part) = match rest.split_once('-') {
            Some((kind, part)) => (kind, Some(number(part).ok_or_else(not)?)),
            None => (rest, None),
        };
        if setup.is_empty() { return Err(not()); }
        Ok(OpId { setup: setup.into(), number: number(n).ok_or_else(not)?, kind: kind.parse().map_err(|_| not())?, part })
    }
}

impl fmt::Display for OpId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}-{}-{}", self.setup, self.number, self.kind)?;
        match self.part {
            Some(part) => write!(f, "-{part}"),
            None => Ok(()),
        }
    }
}

impl TryFrom<String> for OpId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> { s.parse() }
}

impl From<OpId> for String {
    fn from(id: OpId) -> String { id.to_string() }
}

/// An operation id's parts, written as an object rather than as the id's text, for a reader that
/// would otherwise split the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpRef {
    pub setup: String,
    pub number: usize,
    pub kind: OpKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<usize>,
}

impl From<&OpId> for OpRef {
    fn from(id: &OpId) -> OpRef { OpRef { setup: id.setup.clone(), number: id.number, kind: id.kind, part: id.part } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_with_and_without_a_part() {
        for text in ["top-2-roughing", "top-3-spot_drill-2", "bottom-10-v_channel", "front-1-edge_chamfer"] {
            let id: OpId = text.parse().unwrap();
            assert_eq!(id.to_string(), text);
            assert_eq!(serde_json::to_value(&id).unwrap(), serde_json::json!(text));
            assert_eq!(serde_json::from_value::<OpId>(serde_json::json!(text)).unwrap(), id);
        }
        assert_eq!("top-3-spot_drill-2".parse::<OpId>().unwrap(), OpId { setup: "top".into(), number: 3, kind: OpKind::SpotDrill, part: Some(2) });
        assert_eq!("top-2-roughing".parse::<OpId>().unwrap().part, None);
    }

    #[test]
    fn an_op_ref_writes_the_parts_and_a_part_only_when_there_is_one() {
        let parts = |text: &str| serde_json::to_value(OpRef::from(&text.parse::<OpId>().unwrap())).unwrap();
        assert_eq!(parts("top-3-spot_drill-2"), serde_json::json!({"setup": "top", "number": 3, "kind": "spot_drill", "part": 2}));
        assert_eq!(parts("bottom-10-v_channel"), serde_json::json!({"setup": "bottom", "number": 10, "kind": "v_channel"}));
    }

    #[test]
    fn text_that_is_not_an_id_is_refused() {
        for text in ["", "top", "top-2", "top-2-", "-2-roughing", "top-x-roughing", "top-02-roughing", "top-+2-roughing",
            "top-2-pocket", "top-2-roughing-", "top-2-roughing-x", "top-2-roughing-01", "top-2-roughing-2-1", "top:wall-2", "top:v_channel"] {
            assert!(text.parse::<OpId>().is_err(), "{text:?} parsed");
            assert!(serde_json::from_value::<OpId>(serde_json::json!(text)).is_err(), "{text:?} decoded");
        }
    }

    #[test]
    fn kinds_parse_from_their_names() {
        for kind in OpKind::ALL {
            assert_eq!(kind.name().parse::<OpKind>(), Ok(kind));
        }
        assert!("pocket".parse::<OpKind>().is_err());
    }
}
