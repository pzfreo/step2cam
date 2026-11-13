//! A process plan, as Rust types: the plan, its setups, each setup's operations, the features it
//! leaves out and the tools it still needs. Their JSON form is the plan's
//! (`schemas/plan-1.schema.json`), key for key and number for number: a plan decoded into [`Plan`]
//! and encoded again is the same JSON text. Nothing builds or reads a plan through these types yet.
//!
//! Where a field may be missing, its doc comment says which of four rules it keeps:
//! 1. *optional, never null*: absent or a value;
//! 2. *required, nullable*: always written, `null` when there is none;
//! 3. *absent or null*: absent, `null` or a value, all three written today (`Option<Option<T>>`);
//! 4. *pass-through*: any JSON, `null` included, or absent.
//!
//! What stays JSON (the analysis's records, decisions, clamping, materials, meshes and the like) is
//! a `serde_json::Value` here.
use crate::kind::{OpId, OpKind};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Reads a field that is present, so that `null` becomes `Some(None)` (rule 3) or `Some(Null)`
/// (rule 4), and only an absent field is `None`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// A point in a setup's XY plane.
pub type Point2 = [f64; 2];
/// A point in 3D: a setup's or the model's coordinates.
pub type Point3 = [f64; 3];
/// A vertex of a closed outline as camlib takes it: X, Y and the bulge of the arc that follows it
/// (0 for a straight edge).
pub type Vertex = [f64; 3];

/// An axis-aligned box: the part, or a setup's stock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Box3 {
    /// The low corner.
    pub min: Point3,
    /// The high corner.
    pub max: Point3,
}

/// A process plan: the setups that cut the part, and what it leaves out and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// `specify-cam-plan`. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// The plan format's version, 1. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// `mm`. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    /// The analysis's binding to its STEP file. Rule 4: the planner writes `null` when the
    /// analysis has none.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    /// The part's bounds in the model's coordinates. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<Box3>,
    /// The setups, in the order they are cut.
    pub setups: Vec<Setup>,
    /// Features roughing cuts with no operation of their own.
    pub covered_by_roughing: Vec<String>,
    /// Features not cut, and why.
    pub skipped: Vec<Skipped>,
    /// Warnings, as text.
    pub warnings: Vec<String>,
    /// The decision trace. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<Value>>,
    /// The material as specified and as used. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<Value>,
    /// Which rulebase made the plan. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rulebase: Option<Value>,
    /// Tools the library lacks, each with what it would cut. Rule 1: written by `finish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_needed: Option<Vec<Need>>,
    /// How deep roughing must reach for each feature it covers. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughing_feature_depths: Option<BTreeMap<String, Depth>>,
    /// Decisions supplied from outside, and what became of them. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplied_decisions: Option<Vec<Value>>,
    /// The workholding chosen and the candidates considered. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workholding: Option<Value>,
    /// The part's mesh triangles in the model's coordinates, for workholding checks. Rule 4:
    /// `null` without a mesh; the adapter removes it from its output.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub workholding_surface_model: Option<Value>,
    /// The triangles of the part's planar faces, the only certain contact surfaces. Rule 4, as
    /// `workholding_surface_model`.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub contact_surface_model: Option<Value>,
}

/// One setup: the part held one way up, its stock, and the operations cut in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    /// The face that is up: `top`, `bottom`, `front`, `back`, `left` or `right`.
    pub id: String,
    /// What the setup's zero is on: `stock` or `part`. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero: Option<String>,
    /// The rotation from the model's axes to the setup's, by rows; each entry -1, 0 or 1. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[[i8; 3]; 3]>,
    /// The setup's origin in the model's coordinates. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_model: Option<Point3>,
    /// The stock in the setup's coordinates.
    pub stock: Box3,
    /// The operations, in the order they are cut.
    pub operations: Vec<Operation>,
    /// The machine that cuts it. Rule 1: written by `finish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// What the stock is assumed to be when it is not the plan's own. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_note: Option<String>,
    /// `sacrificial_grip` for the top setup of a flip. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_role: Option<String>,
    /// The roughing kept as the part's target surface when roughing itself is not cut here (taken
    /// out by settling, or the bottom setup of a flip). Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_surface: Option<Box<Operation>>,
    /// How the vise holds the stock. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamping: Option<Value>,
    /// How a flipped part sits on its faced grip. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_seating: Option<Value>,
    /// What must be done to the part between the previous setup and this one. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_setup: Option<Value>,
    /// The part's mesh triangles for gripping in a later setup. Rule 4: the plan's
    /// `workholding_surface_model`, `null` included.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub grip_surface_model: Option<Value>,
}

/// One operation: what is cut, and once the plan is finished, with which tool and how.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    /// The cut, by kind.
    #[serde(flatten)]
    pub cut: Cut,
    /// The tool's id. Rule 3: absent until the tool is resolved, then the id, or `null` when no
    /// tool in the library can cut it.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub tool: Option<Option<String>>,
    /// Why there is no tool. Rule 3, as `tool`: `null` when there is one.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub tool_note: Option<Option<String>>,
    /// The decisions behind it, by id. Rule 1; may be empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub because: Option<Vec<String>>,
    /// The ISO 14649 feature and operation it is. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iso14649: Option<Value>,
    /// Feeds, speeds and steps, as the library or its cutting data gives them; integers stay
    /// integers. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technology: Option<Map<String, Value>>,
}

/// What an operation cuts, by its `kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cut {
    /// Faces the stock's top down to the part's.
    Facing {
        /// The faced top.
        floor_z: f64,
    },
    /// Roughs the part out of its stock.
    Roughing(Roughing),
    /// Finishes the part's upward flats between facing and the roughing floor.
    Flats {
        /// The roughing floor.
        floor_z: f64,
        /// The flats' heights, lowest first.
        levels: Vec<f64>,
    },
    /// Spot drills hole mouths.
    SpotDrill(SpotDrill),
    /// Drills or bores holes of one size.
    Holes(Holes),
    /// Cuts one step of a counterbore or spotface.
    Counterbore(Counterbore),
    /// Threads holes.
    Thread(Thread),
    /// Cuts a straight slot.
    Slot(Slot),
    /// Cuts a rectangular recess.
    Notch(Notch),
    /// Cuts a V-channel with a V-bit.
    VChannel(VChannel),
    /// Finishes an outside wall.
    WallFinish(WallFinish),
    /// Chamfers edges, or cuts edge rounds as chamfers.
    EdgeChamfer(EdgeChamfer),
}

impl Cut {
    /// The cut's kind.
    pub fn kind(&self) -> OpKind {
        match self {
            Cut::Facing { .. } => OpKind::Facing,
            Cut::Roughing(_) => OpKind::Roughing,
            Cut::Flats { .. } => OpKind::Flats,
            Cut::SpotDrill(_) => OpKind::SpotDrill,
            Cut::Holes(_) => OpKind::Holes,
            Cut::Counterbore(_) => OpKind::Counterbore,
            Cut::Thread(_) => OpKind::Thread,
            Cut::Slot(_) => OpKind::Slot,
            Cut::Notch(_) => OpKind::Notch,
            Cut::VChannel(_) => OpKind::VChannel,
            Cut::WallFinish(_) => OpKind::WallFinish,
            Cut::EdgeChamfer(_) => OpKind::EdgeChamfer,
        }
    }

    /// The features it cuts, for the kinds that name them.
    pub fn features(&self) -> Option<&[String]> {
        match self {
            Cut::Facing { .. } | Cut::Roughing(_) | Cut::Flats { .. } | Cut::WallFinish(_) => None,
            Cut::SpotDrill(c) => Some(&c.features),
            Cut::Holes(c) => Some(&c.features),
            Cut::Counterbore(c) => Some(&c.features),
            Cut::Thread(c) => Some(&c.features),
            Cut::Slot(c) => Some(&c.features),
            Cut::Notch(c) => Some(&c.features),
            Cut::VChannel(c) => Some(&c.features),
            Cut::EdgeChamfer(c) => Some(&c.features),
        }
    }
}

/// Roughing: down to a floor above the part's bottom, leaving the part's surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Roughing {
    /// Classified access of every roughing region. Absent means unknown; a
    /// stock-edge classification must cover all regions, not just the first entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_access: Option<String>,
    /// The stock's top.
    pub top_z: f64,
    /// The lowest roughing layer.
    pub floor_z: f64,
    /// The material left under the floor, above the part's bottom.
    pub web_mm: f64,
    /// How far the mesh strays from the part's curved faces, rounded up.
    pub surface_error_mm: f64,
    /// The part's mesh in the setup's coordinates, flat XYZ. Rule 1: the adapter removes it from
    /// its output, with `indices`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positions: Option<Vec<f64>>,
    /// The mesh's triangles, three indices into `positions` each. Rule 1, with `positions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indices: Option<Vec<u64>>,
}

/// Spot drilling: one spot tool's spots, before the holes they start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpotDrill {
    /// The spot drill's id. Rule 2: `null` when the library has none that fits.
    pub spot_tool: Option<String>,
    /// The widest spot.
    pub diameter: f64,
    /// The largest drill point angle the spots serve.
    pub angle: f64,
    /// The deepest mouth below the stock's top.
    pub reach: f64,
    /// The holes spotted.
    #[serde(default)]
    pub features: Vec<String>,
    /// The spots, one per hole.
    pub spots: Vec<Spot>,
}

/// One spot at a hole's mouth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    /// The hole's centre.
    pub center: Point2,
    /// The hole's mouth.
    pub top_z: f64,
    /// The spot's diameter: the drill's, or a countersink's.
    pub diameter: f64,
    /// How deep the spot tool goes. Rule 2: `null` without a spot tool.
    pub depth: Option<f64>,
    /// The hole.
    pub features: Vec<String>,
    /// Whether the spot is the hole's countersink.
    pub countersink: bool,
}

/// Holes of one diameter, mouth, bore top, bottom and exit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Holes {
    /// The bore's diameter.
    pub diameter: f64,
    /// The mouth.
    pub top_z: f64,
    /// Where the bore starts, below any counterbore.
    pub bore_top_z: f64,
    /// The bore's bottom.
    pub bottom_z: f64,
    /// `blind` or `through`.
    pub bottom: String,
    /// The holes' centres.
    pub centers: Vec<Point2>,
    /// The holes.
    #[serde(default)]
    pub features: Vec<String>,
    /// How far a drill may go past a through hole's exit before it meets the part. Rule 3:
    /// through holes only (`null` when nothing lies beyond); absent for blind holes.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub exit_clear_mm: Option<Option<f64>>,
    /// Each hole's countersink record from the analysis, `null` for none. Rule 1: only when one
    /// of the holes has a countersink.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub countersinks: Option<Vec<Option<Value>>>,
}

/// One step of a counterbore or spotface, at the holes that share it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Counterbore {
    /// The step's diameter.
    pub diameter: f64,
    /// The step's top.
    pub top_z: f64,
    /// The step's floor.
    pub floor_z: f64,
    /// The holes' centres.
    pub centers: Vec<Point2>,
    /// The holes.
    #[serde(default)]
    pub features: Vec<String>,
}

/// Threads of one size and depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    /// The size as specified: `M5`.
    pub spec: String,
    /// The major diameter.
    pub major: f64,
    /// The pitch.
    pub pitch: f64,
    /// The bore the thread is cut in.
    pub start_diameter: f64,
    /// The thread's top.
    pub top_z: f64,
    /// The full thread's bottom.
    pub bottom_z: f64,
    /// Whether the hole goes through.
    pub through: bool,
    /// The holes' centres.
    pub centers: Vec<Point2>,
    /// The holes.
    #[serde(default)]
    pub features: Vec<String>,
    /// `right` or `left`; right when absent. Rule 1: the planner never writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand: Option<String>,
    /// As [`Holes::exit_clear_mm`], but rule 1: written only for a through thread with a measured
    /// obstruction, never `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_clear_mm: Option<f64>,
    /// Whether a blind thread's depth is the default one, not asked for, so that a tap may cut
    /// less of it to fit its lead. Rule 1: written only as `true`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default_depth: bool,
}

/// A straight slot from `a` to `b`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// The slot's width.
    pub width: f64,
    /// Its length between end centres.
    pub length: f64,
    /// One end's centre.
    pub a: Point2,
    /// The other end's centre.
    pub b: Point2,
    /// The part's top around it.
    pub top_z: f64,
    /// Its floor.
    pub floor_z: f64,
    /// `blind` or `through`.
    pub bottom: String,
    /// The features it cuts.
    #[serde(default)]
    pub features: Vec<String>,
}

/// A rectangular recess, maybe with rounded corners and an open side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notch {
    /// Its width.
    pub width: f64,
    /// Its length.
    pub length: f64,
    /// Whether one side is open to the part's edge.
    pub open: bool,
    /// Its outline.
    pub outer: Vec<Vertex>,
    /// The model's corner radius. Rule 1: the planner always writes it; readers take 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// How many inside corners it has: 2 or 4. Rule 1: the planner always writes it; readers
    /// take 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_corners: Option<u8>,
    /// The part's top.
    pub top_z: f64,
    /// Its floor.
    pub floor_z: f64,
    /// The top of its walls.
    pub wall_top: f64,
    /// `blind` or `through`.
    pub bottom: String,
    /// The features it cuts.
    #[serde(default)]
    pub features: Vec<String>,
}

/// A V-channel, run past the part at both ends from `a` to `b`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VChannel {
    /// The included angle, in degrees.
    pub angle: f64,
    /// Its width at the top.
    pub width: f64,
    /// Its depth.
    pub depth: f64,
    /// Its length in the part.
    pub length: f64,
    /// One end of the run.
    pub a: Point2,
    /// The other end.
    pub b: Point2,
    /// Its top.
    pub top_z: f64,
    /// Its root.
    pub root_z: f64,
    /// Whether it is cut in a flat below the part's top.
    pub on_flat: bool,
    /// The room beside it for the bit's body. Rule 2: `null` when nothing limits it.
    pub room: Option<f64>,
    /// The features it cuts.
    #[serde(default)]
    pub features: Vec<String>,
}

/// An outside wall's finishing pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallFinish {
    /// The wall's name: `wall-1`.
    pub wall: String,
    /// Its outline.
    pub outer: Vec<Vertex>,
    /// Its floor.
    pub floor_z: f64,
    /// Whether it stands on a flat above the roughing floor.
    pub on_flat: bool,
    /// The room beside it for the cutter. Rule 2: `null` when nothing limits it.
    pub room: Option<f64>,
}

/// Edge chamfers along one boundary of the part: the outer outline, holes in it, or both.
///
/// Two producers make it: edge rounds cut as a chamfer (with `radius`, without `room` and
/// `maximum_diameter`) and modelled chamfers (with `room` and `maximum_diameter`, without
/// `radius`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeChamfer {
    /// The wall whose finishing it needs. Rule 2: `null` when it needs none.
    pub needs_wall: Option<String>,
    /// The features lost if that wall is not finished.
    #[serde(default)]
    pub wall_features: Vec<String>,
    /// The openings that must be cut first.
    #[serde(default)]
    pub needs_openings: Vec<String>,
    /// The edge round's radius. Rule 1: edge rounds only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// The chamfer's leg.
    pub width: f64,
    /// The edge's height.
    pub top_z: f64,
    /// The outer outline.
    pub outer: Vec<Vertex>,
    /// The holes' outlines.
    pub holes: Vec<Vec<Vertex>>,
    /// Which boundaries are chamfered: `outer`, `holes` or `all`.
    pub boundaries: String,
    /// The features it cuts.
    #[serde(default)]
    pub features: Vec<String>,
    /// The widest chamfer mill that fits an inside boundary. Rule 3: modelled chamfers only, `null`
    /// on an outside one.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub maximum_diameter: Option<Option<f64>>,
    /// The room beside an outside boundary. Rule 3: modelled chamfers only, `null` on an inside
    /// one or when nothing limits it.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub room: Option<Option<f64>>,
}

/// A feature not cut, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    /// The feature.
    pub feature: String,
    /// Why, as text.
    pub reason: String,
    /// The decision that skipped it: `d-12`. Rule 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

/// A tool the library lacks, and what it would cut.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Need {
    /// The tool, as text.
    pub description: String,
    /// The features it would cut.
    pub features: Vec<String>,
    /// What it would cut: operation ids (`top-3-holes`), or the subject of the warning that
    /// named it (`top:v_channel`, `top:wall-2`), which is not an operation id.
    pub operations: Vec<String>,
}

/// A tool a decision did not choose, and why: an entry of its `considered`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Considered {
    /// The tool's id.
    pub candidate: String,
    /// Why it was not chosen, as text.
    pub rejected: String,
    /// It qualifies, but another tool is used (`rejected` says which and why). Written only when
    /// true.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub qualifies: bool,
    /// Turned down by a reach condition, which the execution engine checks instead when the tool
    /// is chosen anyway (`rulebase/rules/tools.json`, `"reach": true`). Not written.
    #[serde(skip)]
    pub deferred: bool,
}

impl Considered {
    /// A tool turned down, for the reason given.
    pub(crate) fn rejected(candidate: &str, why: impl Into<String>) -> Self {
        Self { candidate: candidate.into(), rejected: why.into(), qualifies: false, deferred: false }
    }

    /// A tool that qualifies, passed over for the reason given (`qualifies, but …`).
    pub(crate) fn passed_over(candidate: &str, why: impl Into<String>) -> Self {
        Self { candidate: candidate.into(), rejected: why.into(), qualifies: true, deferred: false }
    }
}

/// How deep roughing must reach for one feature it covers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Depth {
    /// The feature's lowest point, less an allowance for the mesh.
    pub floor_z: f64,
    /// The feature's family in the analysis.
    pub family: String,
}

/// An operation an execution engine refused, as [`crate::finish_plan`] takes it out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Exclusion {
    /// The operation, as the engine named it.
    pub operation: OpId,
    /// The engine's own words. Empty when not given.
    #[serde(default)]
    pub message: String,
    /// The tool it was refused with, if known.
    #[serde(default)]
    pub tool: Option<String>,
    /// The decision that chose that tool, if any, which the refusal supersedes.
    #[serde(default)]
    pub selection: Option<Value>,
    /// What the engine found out of reach, when the refusal was about reach.
    #[serde(default)]
    pub reach: Option<ReachEvidence>,
}

/// A reach refusal, as the execution engine reported it. Depths are measured down from
/// `top_z_mm`, the top of the material the limiting part must clear, to `z_mm`, the lowest tip Z
/// the operation needs. Its JSON form is the engine's, field for field. The adapter always
/// writes every field; a field left out reads as `null` (rule 2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReachEvidence {
    /// The operation, as the engine named it.
    #[serde(default)]
    pub operation: Option<String>,
    /// The tool's id.
    #[serde(default)]
    pub tool: Option<String>,
    /// The part of the tool assembly that could not reach.
    pub limit: ReachLimit,
    /// Which holder section, for a holder limit.
    #[serde(default)]
    pub holder_section: Option<usize>,
    pub depth_needed_mm: f64,
    /// How far below that top the tip can go before the limiting part meets it.
    #[serde(default)]
    pub depth_available_mm: Option<f64>,
    #[serde(default)]
    pub top_z_mm: Option<f64>,
    #[serde(default)]
    pub z_mm: Option<f64>,
}

/// The part of a tool assembly whose reach was exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachLimit {
    /// The flutes.
    CuttingLength,
    Neck,
    Shank,
    Holder,
}

impl ReachLimit {
    /// Its JSON name: `cutting_length`, `neck`, `shank` or `holder`.
    pub fn name(self) -> &'static str {
        match self {
            ReachLimit::CuttingLength => "cutting_length",
            ReachLimit::Neck => "neck",
            ReachLimit::Shank => "shank",
            ReachLimit::Holder => "holder",
        }
    }
}
