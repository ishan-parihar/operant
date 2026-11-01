// Ported from codex-rs (mermaid/src/tests.rs + families_tests.rs @ 588a277b),
// MIT License. Two forced substitutions, both documented at the site:
//
//   1. `insta::assert_snapshot!` becomes an exact comparison against the
//      expected output, lifted verbatim from upstream's `.snap` files into
//      `crates/operant-cli/tests/mermaid_fixtures/`. Zero new dependencies is
//      a plan constraint, and a snapshot nobody can diff in review is a
//      weaker guard than a reviewable literal.
//   2. `pretty_assertions::assert_eq` becomes the std macro — nicer diff
//      formatting is not a guard.
//
// Everything else is upstream's, including the corpus of rejected inputs.
// Those rejections are the point: the engine's contract is "anything outside
// the bounded subset fails intact", and each of these names a way a diagram
// could otherwise render wrong.

use super::RenderError;
use super::render;
use unicode_width::UnicodeWidthStr;

#[test]
fn stadium_declarations_and_references() {
    let graph =
        super::parse::parse("flowchart TD", &["A", "A([Ready?])", "A", "A([Ready?])"]).unwrap();
    assert_eq!(
        graph.nodes,
        vec![super::Node {
            id: "A".to_owned(),
            label: "Ready?".to_owned(),
            shape: super::Shape::Stadium,
            declared: true,
            members: Vec::new(),
        }]
    );
}

#[test]
fn stadiums_with_other_shapes_in_every_direction() {
    let mut cases = Vec::new();
    for direction in ["TD", "BT", "LR", "RL"] {
        let source = format!(
            "flowchart {direction}; A([\"请求<br>Ready\"]) -- go --> B[Work<br/>next] & C{{\"Done?<br />Retry?\"}}; B -. no .-> C; C <--> A; B --- A; C -.- B; A <-.-> B"
        );
        let output = render(&source, /*max_width*/ 100).unwrap();
        let width = output.lines().map(UnicodeWidthStr::width).max().unwrap();
        assert_eq!(render(&source, width), Ok(output.clone()));
        assert_eq!(render(&source, width - 1), Err(RenderError::TooWide));
        cases.push(format!("{direction}\n{output}"));
    }
    let joined = cases.join("\n\n");
    // [port-substitution] insta snapshot -> reviewable fixture file. The width
    // assertions above already proved the tightest-fitting property; this pins
    // the exact glyphs so any layout drift is a visible diff. The fixture keeps
    // insta's trailing newline, so the expected value is `output + "\n"`.
    assert_eq!(format!("{joined}\n"), include_str!("fixtures/stadiums.txt"));
}

#[test]
fn equivalent_flowchart_forms_preserve_graph() {
    let quoted = r#"A["Review & confirm;"] -->|"Yes & continue"| B{"Ready?"}
B --> C(["Checkout"])
A[Review & confirm;]"#;
    let unquoted = quoted.replace('"', "");
    assert_eq!(
        super::parse::parse("flowchart TD", &quoted.lines().collect::<Vec<_>>()).unwrap(),
        super::parse::parse("flowchart TD", &unquoted.lines().collect::<Vec<_>>()).unwrap(),
    );
    for (infix, pipe) in [
        ("-- Yes -->", "-->|Yes|"),
        ("-- \"Yes & continue\" -->", "-->|\"Yes & continue\"|"),
        ("-. retry .->", "-.->|retry|"),
    ] {
        assert_eq!(
            super::parse::parse("flowchart", &[&format!("A {infix} B --> C")]).unwrap(),
            super::parse::parse("graph TB", &[&format!("A {pipe} B --> C")]).unwrap(),
        );
    }
    assert_eq!(
        super::parse::parse("graph", &["A[Input & config] & B -- send --> C & D -.-> E"]).unwrap(),
        super::parse::parse(
            "flowchart TD",
            &[
                "A[Input & config]",
                "B",
                "C",
                "D",
                "E",
                "A -->|send| C",
                "A -->|send| D",
                "B -->|send| C",
                "B -->|send| D",
                "C -.-> E",
                "D -.-> E",
            ]
        )
        .unwrap(),
    );
    assert_eq!(
        super::parse::parse("flowchart", &["A--- oB", "A-.- xB"])
            .unwrap()
            .nodes,
        super::parse::parse("flowchart", &["A", "oB", "xB"])
            .unwrap()
            .nodes,
    );
    assert_eq!(
        super::parse::parse("flowchart", &["A --> B & B"]).unwrap(),
        super::parse::parse("flowchart TD", &["A --> B", "A --> B"]).unwrap(),
    );
    assert_eq!(
        render(&format!("graph TD; {quoted}"), /*max_width*/ 180).unwrap(),
        render(&format!("graph TD; {unquoted}"), /*max_width*/ 180).unwrap(),
    );
    let source = r#"graph TD;A["chimpansen hoppar ()[]"] -->|"x | y; z"| B{"x < 3?"};"#;
    let statements = super::syntax::statements(source).unwrap();
    assert_eq!(
        super::parse::parse(statements[0], &statements[1..]).unwrap(),
        super::Graph {
            nodes: vec![
                super::Node {
                    id: "A".into(),
                    label: "chimpansen hoppar ()[]".into(),
                    shape: super::Shape::Rectangle,
                    declared: true,
                    members: Vec::new(),
                },
                super::Node {
                    id: "B".into(),
                    label: "x < 3?".into(),
                    shape: super::Shape::Decision,
                    declared: true,
                    members: Vec::new(),
                },
            ],
            edges: vec![super::Edge::directed(
                /*from*/ 0,
                /*to*/ 1,
                "x | y; z".into()
            )],
            ..super::Graph::default()
        }
    );
    for (quoted, label) in [
        (r#"A["(Database)"]"#, "(Database)"),
        (r#"A["/Input/"]"#, "/Input/"),
    ] {
        let graph = super::parse::parse("flowchart TD", &[quoted]).unwrap();
        assert_eq!(
            graph.nodes,
            vec![super::Node {
                id: "A".to_owned(),
                label: label.to_owned(),
                shape: super::Shape::Rectangle,
                declared: true,
                members: Vec::new(),
            }]
        );
    }
}

#[test]
fn entity_labels_keep_source_fallback() {
    for entity in ["&amp;", "&#38;", "&#x26;", "#9829;", "#semi;"] {
        for source in [
            format!("sequenceDiagram\nA->>B: {entity}"),
            format!("classDiagram\nclass A {{\n{entity}\n}}"),
            format!("stateDiagram-v2; A-->B: {entity}"),
            format!("erDiagram\nA {{\nstring value \"{entity}\"\n}}"),
            format!("flowchart TD; A[\"{entity}\"]"),
            format!("flowchart TD; A -->|\"{entity}\"| B"),
        ] {
            assert_eq!(
                render(&source, /*max_width*/ 100),
                Err(RenderError::Unsupported),
                "{source:?}",
            );
        }
    }
}

#[test]
fn flowchart_ampersands_preserve_statement_separators() {
    let source = "flowchart TD; A[R&D] -->|R&D| B[Review & confirm]; B --> C";
    assert_eq!(
        render(&format!("%% &amp;\n{source}"), /*max_width*/ 100),
        render(&source.replace(';', "\n"), /*max_width*/ 100),
    );
    assert!(
        render(source, /*max_width*/ 100)
            .unwrap()
            .contains("Review & confirm")
    );
}

#[test]
fn quoted_flowchart_labels_reject_malformed_and_unsafe_text() {
    for label in [
        "\"unfinished",
        "unfinished\"",
        "\"embedded\"quote\"",
        "\"\"",
        "\"<b>HTML</b>\"",
        "\"before <b\"",
        "\"one<br/><b>two</b>\"",
        "\"one<br class='x'>two\"",
        "\"\u{1b}\"",
    ] {
        for source in [
            format!("flowchart TD; A[{label}]"),
            format!("flowchart TD; A -->|{label}| B"),
            format!("flowchart TD; A -- {label} --> B"),
        ] {
            assert_eq!(
                render(&source, /*max_width*/ 100),
                Err(RenderError::Unsupported),
                "{source:?}",
            );
        }
    }
}

#[test]
fn branches_merges_and_retry_loop() {
    let source = "flowchart TD\nA[\"Go []; &\"] --> B{\"x < 3?\"}\nB -->|\"y|n\"| C[Reserve]\nB -->|no| D[Waitlist]\nC --> E{Paid?}\nE -->|yes| F[Ship]\nE -->|no| G[Retry payment]\nG --> E\nD --> H[Notify buyer]\nF --> H";
    let output = render(source, /*max_width*/ 100).unwrap();
    assert_eq!(format!("{output}\n"), include_str!("fixtures/branches.txt"));
}

#[test]
fn rejects_partial_or_unsupported_input() {
    for source in [
        "flowchart TD; P --> Q; A[(Database)]",
        "flowchart TD; P --> Q; A[/Input/]",
        r"flowchart TD; P --> Q; A[\Output\]",
        r"flowchart TD; P --> Q; A[/Trapezoid\]",
        r"flowchart TD; P --> Q; A[\Inverse/]",
        "flowchart TD; P --> Q; A[[Subroutine]]",
        "flowchart TD; P --> Q; A{{Hexagon}}",
        r#"flowchart TD; P --> Q; A["`hello **world**`"]"#,
        r#"flowchart TD; P --> Q; A{"`Decision`"}"#,
        r#"flowchart TD; P --> Q; A(["`Stadium`"])"#,
        r#"flowchart TD; P --> Q; A -->|"`Caption`"| B"#,
        "flowchart TD; subgraph X; A; end",
        "flowchart TD; A --> B; garbage syntax",
        "flowchart TD; A --> B &",
        "flowchart TD; A && B",
        "flowchart TD; A -- Yes B",
        "flowchart TD; A -. retry --> B",
        "flowchart TD; A ==> B",
        "flowchart TD; A -- hello --- B --> C",
        "flowchart TD; A -. hello .- B -.-> C",
        "flowchart TD; A -- hello ----> B",
        "flowchart TD; A -. hello ..-> B",
        "flowchart TD; A -- hello o--> B",
        "flowchart TD; A --oB --> C",
        "flowchart TD; A -. hello -.-> B",
        "flowchart TD; A---oB",
        "flowchart TD; A-.-xB",
        "flowchart TD; A[one]; A[two]",
        "flowchart TD; A[<b>HTML</b>]",
        "flowchart TD; A[&#27;]",
        "flowchart TD; A[\u{1b}]",
        "flowchart TD; A[e\u{301}]",
        "flowchart TD; A[👍🏽]",
        "flowchart TD; A[👩\u{200d}💻]",
        "flowchart TD; A[✈\u{fe0f}]",
        "flowchart TD; A[zero\u{200b}width]",
        "flowchart TD; A[left\u{202e}right]",
        "flowchart TD; A[\u{2066}isolated\u{2069}]",
        "flowchart TD; A[unclosed",
        "flowchart TD; click A",
        "flowchart TD; A((circle))",
        "flowchart TD; A(rounded)",
        "flowchart TD; A([unclosed]",
        "flowchart TD; A([unclosed)",
        "flowchart TD; A([label]) trailing",
        "flowchart TD; A([])",
        "flowchart TD; A([nested[label]])",
        "flowchart TD; A([one]); A([two])",
        "flowchart TD; A([same]); A[same]",
        "flowchart TD; A{same}; A([same])",
        "flowchart TD; A -->|unclosed B",
        "flowchart TD; A[لا]",
        "flowchart TD; A -->|yes┐| B",
        "flowchart TD; A -->|one<br/>two| B",
        "flowchart TD; A -- one<br>two --> B",
        "flowchart TD; A[<br/><br />]",
    ] {
        assert_eq!(
            render(source, /*max_width*/ 100),
            Err(RenderError::Unsupported),
            "{source:?}"
        );
    }
}

#[test]
fn source_graph_and_width_limits() {
    let grouped = "A & B & C & D --> E & F & G & H & I & J";
    assert!(render(&format!("graph; {grouped}"), /*max_width*/ 200).is_ok());
    for source in [
        " ".repeat(16 * 1024 + 1),
        format!("graph TD; A[{}]", "x".repeat(41)),
        format!("graph; A -- {} --> B", "x".repeat(41)),
        format!("graph; A --> E; {grouped}"),
        format!("graph; {} --> B", ["A"; 25].join(" & ")),
        format!("graph TD; A[\"{}\"]", "[]".repeat(21)),
        format!("graph TD; A([{}])", "x".repeat(41)),
        format!("graph TD; A[\"ok<br />{}\"]", "界".repeat(21)),
        format!(
            "graph TD; {}",
            (0..17).map(|n| format!("N{n};")).collect::<String>()
        ),
        format!("graph TD; {}", "A-->B;".repeat(25)),
    ] {
        assert_eq!(render(&source, /*max_width*/ 200), Err(RenderError::Limit));
    }
    let source = format!(
        "graph LR; A[\"{}<br/>{}\"] --> B[Done]",
        "界".repeat(20),
        "x".repeat(40)
    );
    let output = render(&source, /*max_width*/ 100).unwrap();
    let width = output.lines().map(UnicodeWidthStr::width).max().unwrap();
    assert_eq!(render(&source, width), Ok(output));
    assert_eq!(render(&source, width - 1), Err(RenderError::TooWide));
    assert_eq!(
        render("graph TD; A", /*max_width*/ 0),
        Err(RenderError::TooWide)
    );
}

#[test]
fn reconstruct_every_edge_from_rendered_paths() {
    // All 512 directed graphs on three nodes, including self-loops, cycles, fan-in and fan-out.
    // Reconstruct connections from the emitted glyphs without consulting the renderer's layout.
    for (mask, direction) in
        (0u16..512).flat_map(|mask| ["TD", "BT", "LR", "RL"].map(|direction| (mask, direction)))
    {
        let mut source = format!("graph {direction}; A; B; C;");
        let mut expected = Vec::new();
        for from in 0..3 {
            for to in 0..3 {
                if mask & (1 << (from * 3 + to)) != 0 {
                    let a = char::from(b'A' + from);
                    let b = char::from(b'A' + to);
                    source.push_str(&format!("{a}-->{b};"));
                    expected.push((a, b));
                }
            }
        }
        let output = render(&source, /*max_width*/ 100).unwrap();
        let mut rows = output
            .lines()
            .map(|line| line.chars().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        if matches!(direction, "LR" | "RL") {
            let width = rows.iter().map(Vec::len).max().unwrap();
            rows = (0..width)
                .map(|x| {
                    rows.iter()
                        .map(|row| match row.get(x).copied().unwrap_or(' ') {
                            '─' => '│',
                            '│' => '─',
                            '┐' => '└',
                            '└' => '┐',
                            '┬' => '├',
                            '▲' => '◄',
                            other => other,
                        })
                        .collect()
                })
                .collect();
        }
        let mut order = Vec::new();
        let mut owners = vec![' '; rows.len()];
        for (top, row) in rows.iter().enumerate() {
            if row.first() != Some(&'┌') {
                continue;
            }
            let bottom = (top + 1..rows.len())
                .find(|y| rows[*y].first() == Some(&'└'))
                .unwrap();
            let owner = rows[top..=bottom]
                .iter()
                .flatten()
                .find(|ch| matches!(ch, 'A' | 'B' | 'C'))
                .unwrap();
            owners[top..=bottom].fill(*owner);
            order.push(*owner);
        }
        assert_eq!(
            order,
            if matches!(direction, "BT" | "RL") {
                vec!['C', 'B', 'A']
            } else {
                vec!['A', 'B', 'C']
            }
        );
        let mut actual = Vec::new();
        for (y, row) in rows.iter().enumerate() {
            if let Some(port) = row.windows(2).position(|pair| pair == ['├', '─']) {
                let lane = (port + 1..row.len())
                    .find(|x| matches!(row[*x], '┐' | '┘'))
                    .unwrap();
                let mut target = y;
                loop {
                    target = if row[lane] == '┐' {
                        target + 1
                    } else {
                        target - 1
                    };
                    let ch = rows[target][lane];
                    if matches!(ch, '┘' | '┐') {
                        break;
                    }
                    assert!(matches!(ch, '│' | '╪'), "broken vertical path: {source}");
                }
                assert_eq!(&rows[target][port..port + 2], &['├', '◄']);
                assert!(
                    rows[target][port + 2..lane]
                        .iter()
                        .all(|ch| matches!(ch, '─' | '╪'))
                );
                actual.push((owners[y], owners[target]));
            }
        }
        actual.sort_unstable();
        assert_eq!(actual, expected, "{source}");
    }
}

#[test]
fn semantic_spans_distinguish_labels_from_matching_endpoint_glyphs() {
    use super::Role;

    let lines = super::render_spans("sequenceDiagram; A-xB: x", /*max_width*/ 100).unwrap();
    let roles = lines
        .iter()
        .flatten()
        .flat_map(|span| span.text.chars().filter(|ch| *ch == 'x').map(|_| span.role))
        .collect::<Vec<_>>();
    assert_eq!(roles, vec![Role::Text, Role::Edge]);
    assert_eq!(
        lines[0]
            .iter()
            .find(|span| span.text.contains('┌'))
            .unwrap()
            .role,
        Role::Node
    );
    for direction in ["TD", "BT", "LR", "RL"] {
        let lines = super::render_spans(
            &format!("flowchart {direction}; A --> B"),
            /*max_width*/ 100,
        )
        .unwrap();
        let ports = lines
            .iter()
            .flatten()
            .flat_map(|span| {
                span.text
                    .chars()
                    .filter(|ch| matches!(ch, '├' | '┬'))
                    .map(|_| span.role)
            })
            .collect::<Vec<_>>();
        assert_eq!(ports, vec![Role::Node, Role::Node]);
    }
}

const SEQUENCE: &str = "sequenceDiagram
    actor U as Buyer
    participant A as API
    participant S as 库存
    participant P as Payments
    U->>A: data[0] = {x: 1}
    A->>S: Reserve items
    S-->>A: say \"hello\" | C:\\tmp
    opt Items reserved
        loop Up to 3 attempts
            A->>P: Charge card
            P->>P: Check fraud
            P-->>A: Payment status
            alt Approved
                Note over A,P: Payment recorded
                A-->>U: Order confirmed
            else Declined
                A->>S: Release items
                A-->>U: Payment failed
            end
        end
    end";

const STATE: &str = "stateDiagram-v2
    state \"Payment pending\" as Charging
    [*] --> Draft
    Draft --> Validating: submit
    Validating --> Charging: [ready]
    Validating --> Rejected: invalid
    Charging --> Packing: paid
    Charging --> Rejected: declined
    Packing --> Shipped: dispatch
    Shipped --> Delivered: received
    Delivered --> [*]
    Rejected --> Draft: revise
    Charging: Retry up to 3 times
    Draft: Order drafted
    Draft: Awaiting submission";

const CLASS: &str = "classDiagram
    class order {
        +int[] ids
        +String a;b
        +submit()
        +cancel()
    }
    class LineItem {
        +int quantity
        +Decimal price
        +subtotal()
    }
    class Payment {
        +Decimal amount
        +authorize()
    }
    class CardPayment {
        +String lastFour
        +authorize()
    }
    order \"1\" *-- \"1..*\" LineItem : contains
    order \"1\" --> \"1\" Payment : pays with
    Payment <|-- CardPayment
    CardPayment ..order : updates";

const ER: &str = "erDiagram
    CUSTOMER ||--o{ ORDER : places
    ORDER ||--|{ LINE_ITEM : contains
    PRODUCT ||..o{ LINE_ITEM : appears_in
    CUSTOMER {
        int id PK
        string email UK
        string name
    }
    ORDER {
        int id PK
        int customer_id FK
        string status
    }
    LINE_ITEM {
        int order_id PK, FK \"order key\"
        int product_id PK, FK
        int quantity
    }
    PRODUCT {
        int id PK
        string name
        decimal price
    }";

#[test]
fn complex_families() {
    for (name, source) in [
        ("sequence", SEQUENCE),
        ("state", STATE),
        ("class", CLASS),
        ("er", ER),
    ] {
        let output = render(source, /*max_width*/ 180).unwrap();
        let width = output.lines().map(UnicodeWidthStr::width).max().unwrap();
        assert_eq!(render(source, width), Ok(output.clone()), "{name}");
        assert_eq!(
            render(source, width - 1),
            Err(RenderError::TooWide),
            "{name}"
        );
        // [port-substitution] insta snapshot -> reviewable fixture file.
        let expected = match name {
            "sequence" => include_str!("fixtures/sequence.txt"),
            "state" => include_str!("fixtures/state.txt"),
            "class" => include_str!("fixtures/class.txt"),
            _ => include_str!("fixtures/er.txt"),
        };
        assert_eq!(format!("{output}\n"), expected, "{name}");
    }
}

#[test]
fn unicode_labels_and_later_declarations() {
    for direction in ["TD", "BT", "LR", "RL"] {
        let source = format!(
            "%% heading\ngraph {direction}; A -->|准备| B; A[请求]; B{{Réponse?}}; B -->|retry| A; B --> C[Ship 🚀]"
        );
        let output = render(&source, /*max_width*/ 160).unwrap();
        for label in ["请求", "Réponse?", "Ship 🚀", "准备", "retry"] {
            assert!(output.contains(label), "{direction}: {label}");
        }
        let width = output.lines().map(UnicodeWidthStr::width).max().unwrap();
        assert_eq!(render(&source, width), Ok(output.clone()));
        assert_eq!(render(&source, width - 1), Err(RenderError::TooWide));
        if direction == "LR" {
            assert_eq!(format!("{output}\n"), include_str!("fixtures/LR.txt"));
        }
    }
    for descriptions in [
        vec!["S: Ready"],
        vec!["S: Ready", "S: Working"],
        vec![r#"state "Ready" as S"#, "S: Working"],
        vec!["S: Ready", r#"state "Working" as S"#],
        vec![r#"state "Ready" as S"#, r#"state "Working" as S"#],
    ] {
        // References before and after descriptions must retain the same state identity.
        let mut body = vec!["S --> T"];
        body.extend(&descriptions);
        body.push("T --> S");
        let graph = super::state::parse(&body).unwrap();
        let mut expected = super::state::parse(&["S --> T", "T --> S"]).unwrap();
        expected.nodes[0] = super::Node {
            id: "S".to_owned(),
            label: "Ready".to_owned(),
            shape: super::Shape::Rectangle,
            declared: true,
            members: if descriptions.len() == 1 {
                Vec::new()
            } else {
                vec!["Working".to_owned()]
            },
        };
        assert_eq!(graph, expected, "{descriptions:?}");
    }
}

#[test]
fn graph_relationship_endpoints() {
    for (operator, source_tip, target_tip, dashed) in [
        ("<|--", '◁', '─', false),
        ("*--", '◆', '─', false),
        ("o--", '◇', '─', false),
        ("-->", '─', '◄', false),
        ("--", '─', '─', false),
        ("..>", '─', '◄', true),
        ("..|>", '─', '◁', true),
        ("..", '─', '─', true),
        ("<--", '◄', '─', false),
        ("--*", '─', '◆', false),
        ("--o", '─', '◇', false),
        ("--|>", '─', '◁', false),
    ] {
        let output = render(
            &format!("classDiagram; A \"one\" {operator} \"many\" B : uses"),
            /*max_width*/ 100,
        )
        .unwrap();
        let ports = output
            .lines()
            .filter(|line| line.contains('├'))
            .collect::<Vec<_>>();
        assert!(
            ports[0].contains(&format!("├{source_tip}")),
            "{operator}: {output}"
        );
        assert!(
            ports[1].contains(&format!("├{target_tip}")),
            "{operator}: {output}"
        );
        assert!(ports[0].contains("(one) uses"));
        assert!(ports[1].contains("(many)"));
        assert_eq!(output.contains('┆'), dashed);
    }
    for (operator, source_tip, target_tip, dashed) in [
        ("---", '─', '─', false),
        ("-.->", '─', '◄', true),
        ("-.-", '─', '─', true),
        ("<-->", '◄', '◄', false),
        ("<-.->", '◄', '◄', true),
    ] {
        let output = render(
            &format!("flowchart; A{operator}|uses|B"),
            /*max_width*/ 100,
        )
        .unwrap();
        let ports = output
            .lines()
            .filter(|line| line.contains('├'))
            .collect::<Vec<_>>();
        assert!(
            ports[0].contains(&format!("├{source_tip}")),
            "{operator}: {output}"
        );
        assert!(
            ports[1].contains(&format!("├{target_tip}")),
            "{operator}: {output}"
        );
        assert!(ports[0].contains("uses"));
        assert_eq!(output.contains('┆'), dashed);
    }
    for source in [
        "classDiagram\nclass A; A --> B : {ok}",
        "classDiagram\nclass A; class B {\n+id\n}",
    ] {
        assert_eq!(
            render(source, /*max_width*/ 100).unwrap(),
            render(&source.replace(';', "\n"), /*max_width*/ 100).unwrap(),
        );
    }
    let source = "classDiagram\nclass A {\nstring a;b\nObject[] elementData\n}\nA : +get()";
    let statements = super::syntax::statements(source).unwrap();
    assert_eq!(
        super::relations::parse(statements[0], &statements[1..]).unwrap(),
        super::Graph {
            nodes: vec![super::Node {
                id: "A".into(),
                label: "A".into(),
                shape: super::Shape::Rectangle,
                declared: false,
                members: vec![
                    "string a;b".into(),
                    "Object[] elementData".into(),
                    "+get()".into()
                ],
            }],
            ..super::Graph::default()
        }
    );
    for (line, target, target_tip, target_label, dashed) in [
        ("A --orange", "orange", '─', "", false),
        ("A ..o_range", "o_range", '─', "", true),
        ("A --o B", "B", '◇', "", false),
        (r#"A ..o"many" B"#, "B", '◇', "(many)", true),
    ] {
        let graph = super::relations::parse("classDiagram", &[line]).unwrap();
        let mut expected = super::Graph::default();
        let from = expected.node("A").unwrap();
        let to = expected.node(target).unwrap();
        expected.edges.push(super::Edge {
            from,
            to,
            label: String::new(),
            target_label: target_label.to_owned(),
            source_tip: '─',
            target_tip,
            dashed,
        });
        assert_eq!(graph, expected, "{line}");
    }
}

#[test]
fn er_cardinalities() {
    for (left, source_card) in [
        ("||", "1"),
        ("|o", "0..1"),
        ("}|", "1..many"),
        ("}o", "0..many"),
    ] {
        for (right, target_card) in [
            ("||", "1"),
            ("o|", "0..1"),
            ("|{", "1..many"),
            ("o{", "0..many"),
        ] {
            let output = render(
                &format!("erDiagram; A {left}--{right} B : owns"),
                /*max_width*/ 100,
            )
            .unwrap();
            let ports = output
                .lines()
                .filter(|line| line.contains('├'))
                .collect::<Vec<_>>();
            assert!(ports[0].contains(&format!("({source_card}) owns")));
            assert!(ports[1].contains(&format!("({target_card})")));
        }
    }
}

#[test]
fn rejects_incomplete_and_unsupported_families() {
    for source in [
        "sequenceDiagram; A->>B: hello; nonsense",
        "sequenceDiagram; A->>B: hello; activate B",
        "sequenceDiagram; participant A as x; participant A as y",
        "sequenceDiagram; alt ready; A->>B: hi",
        "sequenceDiagram; A->>B: hi; end",
        "sequenceDiagram; loop retry; else no; end",
        "sequenceDiagram; alt one; else two; else three; end",
        "sequenceDiagram; A->>B: <br/>",
        "sequenceDiagram; A->>B: \u{1b}[31m",
        "sequenceDiagram; participant A as e\u{301}",
        "stateDiagram-v2; state Processing {; A-->B; }",
        "stateDiagram-v2; A --> B: ok; garbage text",
        "stateDiagram-v2; accDescr: Order lifecycle; [*] --> Ready",
        "stateDiagram; ACCDESCR: Order lifecycle; [*] --> Ready",
        "stateDiagram-v2; A:::highlight; A --> B",
        "stateDiagram-v2; A --> B:::highlight",
        "classDiagram; accTitle: Account model; class Account",
        "classDiagram; class A {; +foo()",
        "classDiagram; class A; A --? B",
        "classDiagram; A --> B; click A",
        "classDiagram; class A {; +<html>; }",
        "erDiagram; A ||--o{ B",
        "erDiagram; A ||--|| B: \"owns\"",
        "erDiagram; A {; string value \"x\"junk\"; }",
        "classDiagram\nclass A {\nstring a;b }\n}",
        "erDiagram; A ||--?? B : owns",
        "erDiagram; A {; int x KEY; }",
        "erDiagram; A {; int x PK \"open comment; }",
        "erDiagram; A {; int x; }; A ||--|| B : uses; trailing junk",
        "erDiagram; accDescr {; A model; }; CUSTOMER",
        "erDiagram; ACCDESCR {; A model; }; CUSTOMER",
        "erDiagram; A ||--|| B:::highlight : owns",
        "%%{init: {}}%%\nclassDiagram; class A",
    ] {
        assert_eq!(
            render(source, /*max_width*/ 200),
            Err(RenderError::Unsupported),
            "{source}"
        );
    }
}

#[test]
fn metadata_names_and_style_text_remain_valid_in_content() {
    for source in [
        "classDiagram; class accTitle {\n +id\n }\n class accDescr {\n +id\n }",
        "classDiagram; class A; A: ::: literal; A --> B: ::: literal",
        "erDiagram; accTitle {; int id; }",
        "erDiagram; A ||--|| B: ::: literal",
        "stateDiagram-v2; A: text ::: literal; A --> B: ::: literal",
    ] {
        assert!(render(source, /*max_width*/ 200).is_ok(), "{source}");
    }
}

#[test]
fn family_limits() {
    for source in [
        format!(
            "sequenceDiagram; {}",
            (0..9)
                .map(|i| format!("participant P{i};"))
                .collect::<String>()
        ),
        format!("sequenceDiagram; {}", "A->>B: msg;".repeat(65)),
        format!(
            "sequenceDiagram; {} A->>B: msg; {}",
            "loop retry;".repeat(5),
            "end;".repeat(5)
        ),
        format!("classDiagram; class A {{\n {} }}", "+field\n".repeat(17)),
        format!("erDiagram; A {{; {} }}", "int id;".repeat(17)),
        format!("stateDiagram-v2; {}", "A --> B;".repeat(25)),
        format!("stateDiagram-v2\n{}", "S: Description\n".repeat(18)),
        format!(
            "stateDiagram-v2\n{}",
            "state \"Description\" as S\n".repeat(18)
        ),
    ] {
        assert_eq!(
            render(&source, /*max_width*/ usize::MAX),
            Err(RenderError::Limit),
            "{source}"
        );
    }
}

#[test]
fn truncated_sources_and_terminal_widths() {
    // Exercise incomplete streamed source at every UTF-8 boundary, including inside control blocks.
    for source in [SEQUENCE, STATE, CLASS, ER, "sequenceDiagram; A->>B: 请求"] {
        for end in source.char_indices().map(|(index, _)| index) {
            if let Ok(output) = render(&source[..end], /*max_width*/ 180) {
                assert!(output.lines().all(|line| line.width() <= 180));
                assert!(!output.chars().any(|ch| ch.is_control() && ch != '\n'));
            }
        }
    }
    for source in [
        "sequenceDiagram; A->>B: hi",
        "sequenceDiagram; A->>A: self",
        "sequenceDiagram; Note over A: memo",
        "stateDiagram-v2; state \"a;b\" as A; A-->B: [ready]",
        "erDiagram; A {; string value \"a;b\"; }",
    ] {
        let output = render(source, /*max_width*/ 100).unwrap();
        let width = output.lines().map(UnicodeWidthStr::width).max().unwrap();
        assert_eq!(render(source, width), Ok(output));
        assert_eq!(render(source, width - 1), Err(RenderError::TooWide));
    }
}

#[test]
fn sequence_arrows_preserve_sender_recipient_and_style() {
    for (operator, forward, reverse, dashed) in [
        ("->>", '▶', '◀', false),
        ("-->>", '▶', '◀', true),
        ("->", '─', '─', false),
        ("-->", '┄', '┄', true),
        ("-x", 'x', 'x', false),
        ("--x", 'x', 'x', true),
    ] {
        for (from, to, tip) in [
            ("A", "B", forward),
            ("B", "A", reverse),
            ("B", "B", reverse),
        ] {
            let output = render(
                &format!(
                    "sequenceDiagram; participant A; participant B; {from}{operator}{to}: msg"
                ),
                /*max_width*/ 100,
            )
            .unwrap();
            let rows = output
                .lines()
                .map(|line| line.chars().collect::<Vec<_>>())
                .collect::<Vec<_>>();
            let a = rows[1].iter().position(|ch| *ch == 'A').unwrap();
            let b = rows[1].iter().position(|ch| *ch == 'B').unwrap();
            let recipient = if to == "A" { a } else { b };
            let row = if from == to { 5 } else { 4 };
            assert_eq!(rows[row][recipient], tip, "{from}{operator}{to}");
            assert_eq!(rows[row].contains(&'┄'), dashed);
        }
    }
    let source = "sequenceDiagram; A->>B: say \"hello; B->>A: world\"";
    assert_eq!(
        render(source, /*max_width*/ 180).unwrap(),
        render(&source.replace(';', "\n"), /*max_width*/ 180).unwrap(),
    );
}

#[test]
fn canvas_limit_applies_even_with_unlimited_caller_width() {
    let graph = format!(
        "graph LR; {}",
        format!("A-->|{}|B;", "x".repeat(40)).repeat(24)
    );
    let sequence = format!(
        "sequenceDiagram; {} {}",
        (0..8)
            .map(|i| format!("participant P{i};"))
            .collect::<String>(),
        format!("P0->>P7: {};", "x".repeat(40)).repeat(64)
    );
    for source in [graph, sequence] {
        assert_eq!(
            render(&source, /*max_width*/ usize::MAX),
            Err(RenderError::Limit)
        );
    }
}

// MUTATION GUARDS (codex-architecture-deepen row 5, iter-1601).
//
// The two caps that make a hostile diagram a refusal instead of an OOM or a
// shredded terminal are `RenderError::TooWide` (the width boundary in draw.rs)
// and `RenderError::Limit` (MAX_CELLS). Both guards below were proven dead
// without their cap before being accepted, by applying the mutation and
// observing the named failure:
//
//   guard A: draw.rs `if width > max_width` became `if width + 1 > max_width`.
//     6 tests failed, including
//     `mutation_guard_a_diagram_wider_than_the_terminal_is_refused_not_clipped`
//     at "at exactly the natural width the diagram must still draw".
//   guard B: MAX_CELLS became 64 * 1024 * 1024. 2 tests failed, including
//     `mutation_guard_b_canvas_overrun_is_refused_regardless_of_caller_width`
//     at "MAX_CELLS must cap the canvas even when the caller allows any width".
//
// The engine restored to its unmutated state and both suites are green. If
// either guard is edited, re-derive its failure before trusting it.

#[test]
fn mutation_guard_a_diagram_wider_than_the_terminal_is_refused_not_clipped() {
    // A flowchart whose natural width exceeds the terminal must be refused
    // with TooWide, never silently drawn past the caller's edge (which shreds
    // the terminal) and never clipped mid-node.
    let source = "flowchart LR\nA[aaaaaaaaaaaaaaaaaaaa] --> B[bbbbbbbbbbbbbbbbbbbb]";
    let natural = render(source, /*max_width*/ usize::MAX)
        .expect("the diagram must render with an unbounded width");
    let natural_width = natural
        .lines()
        .map(UnicodeWidthStr::width)
        .max()
        .expect("a rendered diagram has at least one line");

    assert!(
        render(source, natural_width - 1).is_err(),
        "a diagram one column too wide must be refused"
    );
    assert_eq!(
        render(source, natural_width - 1),
        Err(RenderError::TooWide),
        "the refusal must be the width class, not Unsupported or Limit"
    );
    assert_eq!(
        render(source, natural_width),
        Ok(natural),
        "at exactly the natural width the diagram must still draw"
    );
}

#[test]
fn mutation_guard_b_canvas_overrun_is_refused_regardless_of_caller_width() {
    // MAX_CELLS caps the grid the engine will allocate. A caller that says
    // "any width you like" (usize::MAX, which is what the markdown fallback
    // passes when the terminal width is unknown) must not be able to lift that
    // cap: the engine refuses with Limit rather than allocating a grid sized to
    // the caller's optimism.
    //
    // Sizing provenance: each arm below was MEASURED to breach MAX_CELLS
    // (65536) while staying under every other cap, so raising MAX_CELLS flips
    // its answer from `Limit` to a drawing. The graph arm is 20 edges at
    // 40-column labels (1686x46 = 77,556 cells); 18 edges measures
    // 1518x42 = 63,756 and stays under. The sequence arm is 8 participants
    // with 24 messages (MAX_EDGES) whose 3000-column labels push the canvas
    // past the cap: the same shape at 40-column labels measures 324x75 =
    // 24,300 cells and renders fine.
    let wide_graph = format!(
        "graph LR; {}",
        format!("A-->|{}|B; ", "y".repeat(40)).repeat(20)
    );
    let long_sequence = format!(
        "sequenceDiagram; {} {}",
        (0..8)
            .map(|i| format!("participant P{i};"))
            .collect::<String>(),
        format!("P0->>P7: {}; ", "z".repeat(3000)).repeat(24)
    );
    for source in [wide_graph, long_sequence] {
        assert_eq!(
            render(&source, /*max_width*/ usize::MAX),
            Err(RenderError::Limit),
            "MAX_CELLS must cap the canvas even when the caller allows any width"
        );
    }
}
