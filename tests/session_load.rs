//! Turning a session document into split trees: what a loaded window is built from.

use std::path::{Path, PathBuf};

use montty::layout::{Orientation, Shape, SplitTree};
use montty::session::{LeafDoc, SessionDoc, TabDoc, parse, read_session, to_json};

fn leaf(profile: &str) -> Shape<LeafDoc> {
    Shape::Leaf(LeafDoc {
        profile: profile.into(),
        cwd: None,
        command: None,
    })
}

fn doc() -> SessionDoc {
    SessionDoc {
        window: Some((900, 600)),
        active_tab: 1,
        tabs: vec![
            TabDoc {
                title: Some("build".into()),
                focused: 2,
                root: Shape::Split {
                    orientation: Orientation::Horizontal,
                    ratio: 0.25,
                    a: Box::new(leaf("a")),
                    b: Box::new(Shape::Split {
                        orientation: Orientation::Vertical,
                        ratio: 0.75,
                        a: Box::new(leaf("b")),
                        b: Box::new(leaf("c")),
                    }),
                },
            },
            TabDoc {
                title: None,
                focused: 0,
                root: leaf("d"),
            },
        ],
    }
}

#[test]
fn trees_keep_ratios_focus_titles_and_active_tab() {
    let doc = parse(&to_json(&doc()), Path::new("s.json")).unwrap();
    assert_eq!(doc.active_tab, 1);
    assert_eq!(doc.window, Some((900, 600)));
    let mut next = 100;
    let trees: Vec<(SplitTree, usize)> = doc
        .tabs
        .iter()
        .map(|t| {
            let ids = t.root.clone().map(&mut |_| {
                next += 1;
                next
            });
            (SplitTree::from_shape(ids), t.focused)
        })
        .collect();
    let first = trees[0].0.shape();
    let Shape::Split { ratio, b, .. } = &first else {
        panic!("split")
    };
    assert_eq!(*ratio, 0.25);
    let Shape::Split { ratio, .. } = **b else {
        panic!("nested split")
    };
    assert_eq!(ratio, 0.75);
    // Depth-first leaf ids and the focused index pick the focused pane.
    let ids: Vec<u64> = first.leaves().into_iter().copied().collect();
    assert_eq!(ids, [101, 102, 103]);
    assert_eq!(ids[trees[0].1], 103);
    assert_eq!(doc.tabs[0].title.as_deref(), Some("build"));
    assert_eq!(doc.tabs[1].title, None);
    assert_eq!(trees[1].0.panes(), [104]);
}

#[test]
fn read_session_reports_why_it_cannot_open() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("montty-readsession-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sessions")).unwrap();
    std::fs::write(dir.join("sessions/ok.json"), to_json(&doc())).unwrap();
    std::fs::write(dir.join("sessions/bad.json"), "{\n \"version\": 2 }").unwrap();
    assert_eq!(read_session(Some(&dir), "ok").unwrap(), doc());
    let e = read_session(Some(&dir), "bad").unwrap_err();
    assert!(e.starts_with("sessions/bad.json:"), "{e}");
    let e = read_session(Some(&dir), "missing").unwrap_err();
    assert!(e.starts_with("sessions/missing.json: "), "{e}");
    let e = read_session(Some(&dir), "a/b").unwrap_err();
    assert!(e.starts_with("invalid session name: "), "{e}");
    assert!(
        read_session(None, "ok")
            .unwrap_err()
            .contains("no config directory")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
