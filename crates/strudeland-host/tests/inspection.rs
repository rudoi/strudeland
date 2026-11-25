use std::time::Duration;

use strudeland_host::{Host, HostError, InspectionRequest, InspectionView, SourceSpan};
use strudeland_project::InstrumentSnapshot;

const TIMEOUT: Duration = Duration::from_secs(2);

fn song_window(from: u32, to: u32) -> InspectionRequest {
    InspectionRequest {
        view: InspectionView::Song,
        expression: None,
        from,
        to,
        anchor_bar: None,
    }
}

fn expression(code: &str, source: &str) -> InspectionRequest {
    let start = code.find(source).unwrap();
    let from = code[..start].encode_utf16().count() as u32;
    InspectionRequest {
        view: InspectionView::Expression,
        expression: Some(SourceSpan {
            from,
            to: from + source.encode_utf16().count() as u32,
        }),
        ..song_window(0, 1)
    }
}

#[test]
fn an_expression_inspects_its_own_lexical_value_and_source() {
    let host = Host::new().unwrap();
    let code = r#"// 🎹
const motif = note("c4 e4 f4");
const harmony = motif.transpose(12).s("triangle");
s("bd")"#;
    let request = expression(code, r#"motif.transpose(12).s("triangle")"#);
    let result = host
        .inspect(code, &request, &InstrumentSnapshot::new(), TIMEOUT)
        .unwrap();
    assert!(!result.truncated);
    let notes = result
        .notes
        .iter()
        .map(|n| (n.pitch, &*n.start, &*n.end, n.sound.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        notes,
        [
            (72.0, "0", "1/3", Some("triangle")),
            (76.0, "1/3", "2/3", Some("triangle")),
            (77.0, "2/3", "1", Some("triangle")),
        ]
    );
    let source_at = |span: &SourceSpan| {
        String::from_utf16(
            &code.encode_utf16().collect::<Vec<_>>()[span.from as usize..span.to as usize],
        )
        .unwrap()
    };
    assert!(
        result.notes[0]
            .contributors
            .iter()
            .any(|span| source_at(span) == "c4")
    );
    assert!(
        host.inspect(
            code,
            &song_window(0, 1),
            &InstrumentSnapshot::new(),
            TIMEOUT
        )
        .unwrap()
        .notes
        .is_empty()
    );
}

#[test]
fn an_expression_has_local_phase_and_uses_the_written_meter_anchor() {
    let host = Host::new().unwrap();
    let pattern = r#"new Pattern(state => note("<c4 e4>").transpose(state.controls._cps > 0.55 ? 12 : 0).query(state))"#;
    let code = format!(
        r#"const meta = {{
  bpm: 120,
  meter: '4/4',
  sections: [{{ name: 'intro', bars: 1 }}, {{ name: 'outro', bars: 2 }}],
  meterChanges: [{{ bar: 2, meter: '7/8' }}],
}};
const motif = {pattern};
song(meta, {{ intro: motif, outro: motif }})"#
    );
    let anchor = |anchor_bar| InspectionRequest {
        anchor_bar,
        ..expression(&code, pattern)
    };
    for (meter, pitch) in [(None, 60.0), (Some(0), 60.0), (Some(1), 72.0)] {
        let result = host
            .inspect(&code, &anchor(meter), &InstrumentSnapshot::new(), TIMEOUT)
            .unwrap();
        assert_eq!(result.notes.len(), 1);
        assert_eq!(result.notes[0].pitch, pitch);
    }
    let arranged = host
        .inspect(
            &code,
            &song_window(1, 3),
            &InstrumentSnapshot::new(),
            TIMEOUT,
        )
        .unwrap();
    assert_eq!(
        arranged
            .notes
            .iter()
            .map(|n| (n.pitch, n.start.as_str()))
            .collect::<Vec<_>>(),
        [(72.0, "1"), (76.0, "2")]
    );
}

#[test]
fn inspection_rejects_wide_requests_and_recovers_after_a_query_timeout() {
    let host = Host::new().unwrap();
    let pins = InstrumentSnapshot::new();
    let code = r#"note("c4")"#;
    assert!(matches!(
        host.inspect(code, &song_window(0, 17), &pins, TIMEOUT),
        Err(HostError::Plan(_))
    ));
    let times_out = "new Pattern(() => { while (true) {} })";
    assert!(matches!(
        host.inspect(
            times_out,
            &song_window(0, 1),
            &pins,
            Duration::from_millis(250),
        ),
        Err(HostError::Timeout { .. })
    ));
    assert_eq!(
        host.inspect(code, &song_window(0, 1), &pins, TIMEOUT)
            .unwrap()
            .notes[0]
            .pitch,
        60.0
    );
}

#[test]
fn inspection_bounds_reported_eval_and_query_errors() {
    let host = Host::new().unwrap();
    let pins = InstrumentSnapshot::new();
    for (code, query) in [
        (
            "throw new Error('eval error ' + 'é'.repeat(1024 * 1024)); note(60)",
            false,
        ),
        (
            "note(60).withHap(() => { throw new Error('query error ' + 'é'.repeat(1024 * 1024)); })",
            true,
        ),
    ] {
        let error = host
            .inspect(code, &song_window(0, 1), &pins, TIMEOUT)
            .unwrap_err();
        let message = match (query, error) {
            (false, HostError::Eval(message)) | (true, HostError::Plan(message)) => message,
            (_, actual) => panic!("unexpected error: {actual}"),
        };
        assert!(message.len() <= 4_096);
        assert!(message.starts_with(if query { "query error " } else { "eval error " }));
        assert!(message.ends_with('…'));
    }
}
