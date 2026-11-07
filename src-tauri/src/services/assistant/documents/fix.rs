//! The layout problems the host fixes itself after officecli's checks: a
//! slide's text that overflows its box, a shape past the slide's edge, and
//! dim text on a dark fill. Pure: officecli's reports in, `set` commands out
//! (see `create::fix_layout` for the loop that runs them).

use serde_json::{json, Map, Value};
use std::sync::LazyLock;

/// officecli's slide, 16:9.
pub(super) const SLIDE_W_CM: f64 = 33.867;
pub(super) const SLIDE_H_CM: f64 = 19.05;
/// The margin a moved or grown shape keeps from the slide's edge.
pub(super) const EDGE_CM: f64 = 1.27;
/// The smallest text an overflow fix shrinks to, and by how much a round.
pub(super) const MIN_SIZE_PT: f64 = 13.0;
pub(super) const SIZE_STEP_PT: f64 = 2.0;
/// Fix, check again; at most this many times.
pub(super) const MAX_ROUNDS: usize = 2;

const PT_PER_CM: f64 = 28.3465;
const CM_PER_IN: f64 = 2.54;
const EMU_PER_CM: f64 = 360_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// The text needs a box `suggest_cm` high at `size_pt`.
    Overflow {
        size_pt: f64,
        suggest_cm: f64,
    },
    OffSlide {
        edge: Edge,
        by_cm: f64,
    },
    LowContrast,
    Other,
}

/// One problem of `view issues --json` and the element it is on.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ViewIssue {
    pub path: String,
    pub kind: Kind,
}

impl ViewIssue {
    /// Whether its fix depends on where the shape is.
    pub(super) fn needs_geometry(&self) -> bool {
        matches!(self.kind, Kind::Overflow { .. } | Kind::OffSlide { .. })
    }
}

// "text overflow: 7 lines at 20.0pt need 168pt, usable 49pt. suggest.height=6.2cm"
static OVERFLOW: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"text overflow: \d+ lines? at ([\d.]+)pt need [\d.]+pt, usable -?[\d.]+pt\. suggest\.height=([\d.]+)cm",
    )
    .expect("a valid pattern")
});

// "Text shape \"出界\" extends 2.1cm past slide right edge"
static OFF_SLIDE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"extends ([\d.]+)cm past slide (left|right|top|bottom) edge")
        .expect("a valid pattern")
});

/// An issue of officecli's `view issues --json`; `None` without the path of
/// the element it is on.
pub(super) fn parse_issue(issue: &Value) -> Option<ViewIssue> {
    let path = issue["path"].as_str().filter(|path| !path.is_empty())?;
    let message = issue["message"].as_str().unwrap_or_default();
    let number = |text: &str| text.parse::<f64>().ok();
    let kind = if issue["subtype"] == "low_contrast" {
        Kind::LowContrast
    } else if let Some((size_pt, suggest_cm)) = OVERFLOW
        .captures(message)
        .and_then(|found| Some((number(&found[1])?, number(&found[2])?)))
    {
        Kind::Overflow {
            size_pt,
            suggest_cm,
        }
    } else if let Some((by_cm, edge)) = OFF_SLIDE
        .captures(message)
        .and_then(|found| Some((number(&found[1])?, found.get(2)?.as_str())))
    {
        let edge = match edge {
            "left" => Edge::Left,
            "right" => Edge::Right,
            "top" => Edge::Top,
            _ => Edge::Bottom,
        };
        Kind::OffSlide { edge, by_cm }
    } else {
        Kind::Other
    };
    Some(ViewIssue {
        path: path.to_owned(),
        kind,
    })
}

/// Where a shape is on its slide, in centimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A length as officecli writes one (`2.33cm`, `66pt`, `1in`, or EMU as a
/// bare number), in centimetres.
pub(super) fn length_cm(text: &str) -> Option<f64> {
    let text = text.trim();
    let (number, per_cm) = if let Some(number) = text.strip_suffix("cm") {
        (number, 1.0)
    } else if let Some(number) = text.strip_suffix("pt") {
        (number, PT_PER_CM)
    } else if let Some(number) = text.strip_suffix("in") {
        (number, 1.0 / CM_PER_IN)
    } else {
        (text, EMU_PER_CM)
    };
    let value = number.trim().parse::<f64>().ok()?;
    value.is_finite().then_some(value / per_cm)
}

/// The shape's place in officecli's `get <path> --json` report.
pub(super) fn geometry(get_report: &Value) -> Option<Geometry> {
    let format = &get_report["data"]["results"][0]["format"];
    let length = |key: &str| format[key].as_str().and_then(length_cm);
    Some(Geometry {
        x: length("x")?,
        y: length("y")?,
        width: length("width")?,
        height: length("height")?,
    })
}

/// A change to one element: `set <path>` with `props`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fix {
    pub path: String,
    pub props: Map<String, Value>,
}

/// The change that fixes `issue`, if one does: an overflowing box grows to
/// the suggested height when that stays on the page, or else its text
/// shrinks a step, to [`MIN_SIZE_PT`] at the least (none when it is there
/// already); a shape past an edge moves back inside it; dim text turns
/// white. `geometry` is the shape's place.
pub(super) fn fix_for(issue: &ViewIssue, geometry: Option<&Geometry>) -> Option<Fix> {
    let (key, value) = match issue.kind {
        Kind::Overflow {
            size_pt,
            suggest_cm,
        } => {
            if geometry.is_some_and(|at| at.y + suggest_cm <= SLIDE_H_CM - EDGE_CM) {
                ("height", cm(suggest_cm))
            } else if size_pt > MIN_SIZE_PT {
                ("size", number((size_pt - SIZE_STEP_PT).max(MIN_SIZE_PT)))
            } else {
                return None;
            }
        }
        Kind::OffSlide { edge, .. } => {
            let at = geometry?;
            match edge {
                Edge::Right => ("x", cm((SLIDE_W_CM - EDGE_CM - at.width).max(0.0))),
                Edge::Left => ("x", cm(EDGE_CM)),
                Edge::Bottom => ("y", cm((SLIDE_H_CM - EDGE_CM - at.height).max(0.0))),
                Edge::Top => ("y", cm(EDGE_CM)),
            }
        }
        Kind::LowContrast => ("color", "FFFFFF".to_owned()),
        Kind::Other => return None,
    };
    let mut props = Map::new();
    props.insert(key.into(), Value::String(value));
    Some(Fix {
        path: issue.path.clone(),
        props,
    })
}

/// `value` rounded to three decimals, without trailing zeros.
fn number(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    // No "-0".
    format!("{}", rounded + 0.0)
}

fn cm(value: f64) -> String {
    format!("{}cm", number(value))
}

/// `fixes` as officecli batch commands.
pub(super) fn fix_batch(fixes: &[Fix]) -> Vec<Value> {
    fixes
        .iter()
        .map(|fix| json!({"command": "set", "path": fix.path, "props": fix.props}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const OVERFLOW: &str =
        "text overflow: 7 lines at 20.0pt need 168pt, usable 49pt. suggest.height=6.2cm";
    const BODY: &str = "/slide[1]/shape[@id=100000]";

    fn issue(path: &str, message: &str) -> Value {
        json!({"id": "O1", "type": 0, "severity": 1, "path": path, "message": message})
    }

    fn overflow(size_pt: f64, suggest_cm: f64) -> ViewIssue {
        ViewIssue {
            path: BODY.into(),
            kind: Kind::Overflow {
                size_pt,
                suggest_cm,
            },
        }
    }

    fn off_slide(edge: Edge, by_cm: f64) -> ViewIssue {
        ViewIssue {
            path: BODY.into(),
            kind: Kind::OffSlide { edge, by_cm },
        }
    }

    fn at(x: f64, y: f64, width: f64, height: f64) -> Geometry {
        Geometry {
            x,
            y,
            width,
            height,
        }
    }

    fn props(fix: Option<Fix>) -> Value {
        let fix = fix.expect("a fix");
        assert_eq!(fix.path, BODY);
        Value::Object(fix.props)
    }

    fn close(found: Option<f64>, expected: f64) -> bool {
        found.is_some_and(|found| (found - expected).abs() < 0.001)
    }

    #[test]
    fn parses_an_overflow_with_its_suggested_height() {
        let parsed = parse_issue(&issue(BODY, OVERFLOW)).unwrap();
        assert_eq!(parsed.path, BODY);
        assert_eq!(
            parsed.kind,
            Kind::Overflow {
                size_pt: 20.0,
                suggest_cm: 6.2
            }
        );
    }

    #[test]
    fn parses_off_slide_and_low_contrast() {
        let off = parse_issue(&issue(
            "/slide[1]/shape[@id=100001]",
            "Text shape \"出界\" extends 2.1cm past slide right edge",
        ))
        .unwrap();
        assert_eq!(off.path, "/slide[1]/shape[@id=100001]");
        assert_eq!(
            off.kind,
            Kind::OffSlide {
                edge: Edge::Right,
                by_cm: 2.1
            }
        );
        let left = parse_issue(&issue(
            BODY,
            "Text shape \"left off\" extends 2.0cm past slide left edge",
        ))
        .unwrap();
        assert_eq!(
            left.kind,
            Kind::OffSlide {
                edge: Edge::Left,
                by_cm: 2.0
            }
        );
        let dim = parse_issue(&json!({
            "id": "C3", "type": 0, "subtype": "low_contrast", "severity": 1,
            "path": "/slide[1]/shape[@id=100002]",
            "message": "Low-contrast text #555555 on dark fill — unreadable on projection. Use FFFFFF or a color brighter than 80%.",
        }))
        .unwrap();
        assert_eq!(dim.path, "/slide[1]/shape[@id=100002]");
        assert_eq!(dim.kind, Kind::LowContrast);
        let other = parse_issue(&issue(BODY, "Slide has no title")).unwrap();
        assert_eq!(other.kind, Kind::Other);
        // Without a path there is nothing to fix.
        assert!(parse_issue(&json!({"severity": 1, "message": OVERFLOW})).is_none());
        assert!(parse_issue(&issue("", OVERFLOW)).is_none());
    }

    #[test]
    fn length_cm_reads_pt_cm_in_and_emu() {
        assert!(close(length_cm("66pt"), 2.328));
        assert!(close(length_cm("29.21cm"), 29.21));
        assert!(close(length_cm("1in"), 2.54));
        assert!(close(length_cm("360000"), 1.0));
        assert!(length_cm("wide").is_none());
        assert!(length_cm("").is_none());
    }

    #[test]
    fn geometry_reads_the_first_results_format() {
        let report = json!({"success": true, "data": {"matches": 1, "results": [{
            "path": "/slide[1]/shape[@id=2]", "type": "title",
            "format": {"name": "Title", "id": 2, "x": "66pt", "y": "28.75pt", "width": "29.21cm", "height": "104pt"},
        }]}});
        let found = geometry(&report).unwrap();
        assert!(close(Some(found.x), 2.328));
        assert!(close(Some(found.y), 1.014));
        assert!(close(Some(found.width), 29.21));
        assert!(close(Some(found.height), 3.669));
        assert!(geometry(&json!({"success": false})).is_none());
    }

    #[test]
    fn overflow_grows_the_box_when_it_fits_the_page() {
        let fix = fix_for(&overflow(20.0, 6.2), Some(&at(2.33, 5.2, 14.0, 2.0)));
        assert_eq!(props(fix), json!({"height": "6.2cm"}));
    }

    #[test]
    fn overflow_past_the_bottom_steps_the_size_down_to_13() {
        let low = at(2.33, 12.0, 14.0, 2.0);
        assert_eq!(
            props(fix_for(&overflow(20.0, 7.0), Some(&low))),
            json!({"size": "18"})
        );
        assert_eq!(
            props(fix_for(&overflow(15.0, 7.0), Some(&low))),
            json!({"size": "13"})
        );
        assert_eq!(
            props(fix_for(&overflow(14.0, 7.0), Some(&low))),
            json!({"size": "13"})
        );
        assert!(fix_for(&overflow(13.0, 7.0), Some(&low)).is_none());
        assert!(fix_for(&overflow(12.0, 7.0), Some(&low)).is_none());
        // Where the box is unknown, only the size can change.
        assert_eq!(
            props(fix_for(&overflow(20.0, 6.2), None)),
            json!({"size": "18"})
        );
    }

    #[test]
    fn off_slide_clamps_inside_the_edge() {
        let right = at(30.0, 2.0, 6.0, 2.0);
        assert_eq!(
            props(fix_for(&off_slide(Edge::Right, 2.1), Some(&right))),
            json!({"x": "26.597cm"})
        );
        assert_eq!(
            props(fix_for(
                &off_slide(Edge::Left, 2.0),
                Some(&at(-2.0, 10.0, 6.0, 2.0))
            )),
            json!({"x": "1.27cm"})
        );
        assert_eq!(
            props(fix_for(
                &off_slide(Edge::Bottom, 1.9),
                Some(&at(2.0, 18.0, 6.0, 3.0))
            )),
            json!({"y": "14.78cm"})
        );
        assert_eq!(
            props(fix_for(
                &off_slide(Edge::Top, 1.0),
                Some(&at(2.0, -1.0, 6.0, 3.0))
            )),
            json!({"y": "1.27cm"})
        );
        // Wider than the slide: as far left as it goes.
        assert_eq!(
            props(fix_for(
                &off_slide(Edge::Right, 9.0),
                Some(&at(3.0, 2.0, 40.0, 2.0))
            )),
            json!({"x": "0cm"})
        );
        assert!(fix_for(&off_slide(Edge::Right, 2.1), None).is_none());
    }

    #[test]
    fn low_contrast_turns_the_text_white() {
        let dim = ViewIssue {
            path: BODY.into(),
            kind: Kind::LowContrast,
        };
        assert_eq!(props(fix_for(&dim, None)), json!({"color": "FFFFFF"}));
        let other = ViewIssue {
            path: BODY.into(),
            kind: Kind::Other,
        };
        assert!(fix_for(&other, Some(&at(1.0, 1.0, 1.0, 1.0))).is_none());
    }

    #[test]
    fn fix_batch_sets_each_fix() {
        let fixes: Vec<Fix> = [
            fix_for(&overflow(20.0, 6.2), Some(&at(2.33, 5.2, 14.0, 2.0))),
            fix_for(
                &ViewIssue {
                    path: BODY.into(),
                    kind: Kind::LowContrast,
                },
                None,
            ),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert_eq!(
            Value::Array(fix_batch(&fixes)),
            json!([
                {"command": "set", "path": BODY, "props": {"height": "6.2cm"}},
                {"command": "set", "path": BODY, "props": {"color": "FFFFFF"}},
            ])
        );
    }

    #[test]
    fn fix_batches_pass_the_validator() {
        let fixes = [
            fix_for(&overflow(20.0, 6.2), Some(&at(2.33, 5.2, 14.0, 2.0))),
            fix_for(&overflow(20.0, 7.0), Some(&at(2.33, 12.0, 14.0, 2.0))),
            fix_for(&off_slide(Edge::Right, 2.1), Some(&at(30.0, 2.0, 6.0, 2.0))),
            fix_for(
                &off_slide(Edge::Bottom, 1.9),
                Some(&at(2.0, 18.0, 6.0, 3.0)),
            ),
            fix_for(
                &ViewIssue {
                    path: "/slide[2]/shape[@id=2]".into(),
                    kind: Kind::LowContrast,
                },
                None,
            ),
        ];
        for fix in fixes {
            let mut commands = fix_batch(&[fix.expect("a fix")]);
            assert_eq!(
                super::super::create::check_batch(&mut commands),
                Ok(()),
                "{commands:?}"
            );
        }
    }
}
