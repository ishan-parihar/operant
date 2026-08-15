#!/usr/bin/env python3
"""
diagnose.py — Heuristic shadow-signature scanner for SKILL.md files.

This script scans a target skill's SKILL.md (and optionally its bundled
reference files) for surface markers that correlate with each of the five
holonic shadows. It produces a *signal*, not a verdict — the auditor must
hand-classify using execution traces and user complaint.

Usage:
    python diagnose.py <path-to-skill-directory>
    python diagnose.py <path-to-SKILL.md>

Output:
    A JSON report with shadow scores and the markers that triggered them.
"""

import re
import sys
import json
from pathlib import Path


# --- Shadow 1 (Dark-Addiction) markers: Matrix overload ---
S1_MARKERS = [
    (r"\bcomprehensively ingest\b", 3, "explicit Catalyst-flooding directive"),
    (r"\bread all\b.*\bbefore proceeding\b", 3, "blocks action on ingestion"),
    (r"\bbe thorough\b", 1, "vague thoroughness directive"),
    (r"\bcomprehensive(ly)?\b", 1, "comprehensiveness directive"),
    (r"\bdouble[- ]check\b", 1, "re-verification directive"),
    (r"\bre[- ]read\b", 2, "explicit re-read directive"),
    (r"\bverify by re[- ]?reading\b", 3, "re-read-as-verification directive"),
]

# --- Shadow 2 (Dark-Allergy) markers: Matrix starvation ---
S2_MARKERS = [
    (r"\bALWAYS\b|\bNEVER\b|\bMUST\b", 1, "imperative without reasoning (counted, not forbidden)"),
]
# Shadow 2 is mostly diagnosed by ABSENCE — few examples, no "why", short SKILL.md.

# --- Shadow 3 (Golden-Addiction) markers: Potentiator flooding ---
S3_MARKERS = [
    (r"\byou may also\b", 2, "open-ended branch without substrate"),
    (r"\bbe creative\b", 2, "creativity directive without bounding"),
    (r"\bbe comprehensive\b", 2, "comprehensiveness directive (Potentiator side)"),
    (r"\bfeel free to\b", 1, "permission without structure"),
    (r"\bexplore\b.*\bpossibilities\b", 2, "exploration directive"),
]

# --- Shadow 4 (Golden-Allergy) markers: Potentiator stagnation ---
S4_MARKERS = [
    (r"\bstep \d+:", 1, "rigid step-by-step script (counted; high counts suggest rigidity)"),
    (r"\bfollow (this|the) (exact|precise) (procedure|steps|script)\b", 3, "explicit rigidity directive"),
    (r"\bdo not deviate\b", 3, "explicit anti-adaptation directive"),
    (r"\bexactly as (specified|written|shown)\b", 2, "exactness directive"),
]

# --- Shadow 5 (Sinkhole of Indifference) markers: depolarization ---
S5_MARKERS = [
    # Generic capability language in description
    (r"\b(writes|generates|creates|produces|extracts|analyzes)\b.*\b(for|from|on)\b", 1, "generic capability phrasing"),
    # Hedging-permissive language
    (r"\b(may|could|might|depending on|various|multiple)\b", 1, "hedging-permissive language"),
    # Template-feeling sections
    (r"\b(summary|overview|introduction|conclusion)\b.*\bsection\b", 1, "generic section name"),
]


def strip_metalinguistic(text: str) -> str:
    """Strip content where shadow-vocabulary words appear metalinguistically.

    The scanner cannot distinguish directive use ("comprehensively ingest all
    material before proceeding") from metalinguistic use ("the skill says
    'comprehensively ingest' which is a Shadow 1 marker"). Metalinguistic use
    lives in: code blocks, blockquotes, table cells, and inline code.
    Stripping these before scanning eliminates the false-positive problem.
    """
    # Strip fenced code blocks (```...```)
    text = re.sub(r"```[^`]*```", "", text, flags=re.DOTALL)
    # Strip inline code (`...`)
    text = re.sub(r"`[^`]+`", "", text)
    # Strip blockquote lines (> ...)
    text = re.sub(r"^>.*$", "", text, flags=re.MULTILINE)
    # Strip table rows (lines starting with |)
    text = re.sub(r"^\|.*$", "", text, flags=re.MULTILINE)
    return text


def scan_text(text: str, markers: list) -> list:
    """Scan text for markers; return list of (marker, score, description, match) hits."""
    hits = []
    for pattern, score, desc in markers:
        for match in re.finditer(pattern, text, re.IGNORECASE):
            hits.append({
                "marker": pattern,
                "score": score,
                "description": desc,
                "match": match.group(0),
                "position": match.start(),
            })
    return hits


def count_examples(text: str) -> int:
    """Rough count of 'Example' or 'Input/Output' blocks."""
    patterns = [
        r"\bExample \d+:",
        r"\bExample:",
        r"\*\*Example",
        r"\bInput:.*\bOutput:",
    ]
    count = 0
    for p in patterns:
        count += len(re.findall(p, text, re.IGNORECASE))
    return count


def count_why_explanations(text: str) -> int:
    """Rough count of 'why' explanations — sentences with 'because' or 'so that'."""
    patterns = [
        r"\bbecause\b",
        r"\bso that\b",
        r"\bthe reason\b",
        r"\bthis (matters|is important|ensures)\b",
    ]
    count = 0
    for p in patterns:
        count += len(re.findall(p, text, re.IGNORECASE))
    return count


def diagnose_skill(skill_path: str) -> dict:
    """Diagnose a skill directory or SKILL.md file."""
    skill_path = Path(skill_path)

    # Locate SKILL.md
    if skill_path.is_dir():
        skill_md = skill_path / "SKILL.md"
        ref_dir = skill_path / "references"
    else:
        skill_md = skill_path
        ref_dir = skill_path.parent / "references"

    if not skill_md.exists():
        return {"error": f"SKILL.md not found at {skill_md}"}

    skill_text = skill_md.read_text(encoding="utf-8", errors="ignore")
    skill_lines = skill_text.count("\n") + 1

    # Strip metalinguistic content (code blocks, blockquotes, table cells)
    # before scanning — this prevents false-positives where the skill names
    # shadow-vocabulary words inside examples or anti-patterns.
    scan_text_clean = strip_metalinguistic(skill_text)

    # Aggregate reference file text (for overlap detection only)
    ref_texts = []
    ref_files = []
    if ref_dir.exists():
        for ref_file in sorted(ref_dir.glob("*.md")):
            ref_texts.append(ref_file.read_text(encoding="utf-8", errors="ignore"))
            ref_files.append(ref_file.name)

    # --- Shadow 1: Dark-Addiction ---
    s1_hits = scan_text(scan_text_clean, S1_MARKERS)
    s1_score = sum(h["score"] for h in s1_hits)

    # Line-count contribution
    if skill_lines > 500:
        s1_score += 5
        s1_line_note = f"SKILL.md is {skill_lines} lines (>500 → Matrix overload risk)"
    elif skill_lines < 100:
        s1_line_note = f"SKILL.md is {skill_lines} lines (<100 → Matrix starvation risk, see Shadow 2)"
    else:
        s1_line_note = f"SKILL.md is {skill_lines} lines (healthy range)"

    # Reference overlap (rough — counts duplicate headings)
    if ref_texts:
        all_headings = []
        for t in [skill_text] + ref_texts:
            all_headings.extend(re.findall(r"^#+\s+(.+)$", t, re.MULTILINE))
        unique_headings = set(all_headings)
        overlap = len(all_headings) - len(unique_headings)
        if overlap > 5:
            s1_score += 3
            s1_overlap_note = f"{overlap} duplicate headings across skill + references (>5 → Catalyst overlap)"
        else:
            s1_overlap_note = f"{overlap} duplicate headings (low overlap)"
    else:
        s1_overlap_note = "no reference files"

    # --- Shadow 2: Dark-Allergy ---
    s2_hits = scan_text(scan_text_clean, S2_MARKERS)
    s2_score = sum(h["score"] for h in s2_hits)

    example_count = count_examples(scan_text_clean)
    why_count = count_why_explanations(scan_text_clean)

    if skill_lines < 100:
        s2_score += 5
    if example_count < 2:
        s2_score += 3
        s2_example_note = f"{example_count} examples found (<2 → boundary rigidified)"
    else:
        s2_example_note = f"{example_count} examples found (healthy)"
    if why_count < 2:
        s2_score += 2
        s2_why_note = f"{why_count} 'why' explanations found (<2 → rules without reasoning)"
    else:
        s2_why_note = f"{why_count} 'why' explanations found (healthy)"

    # --- Shadow 3: Golden-Addiction ---
    s3_hits = scan_text(scan_text_clean, S3_MARKERS)
    s3_score = sum(h["score"] for h in s3_hits)

    # Check for verifiability fields in output templates
    has_citation_requirement = bool(re.search(r"\b(cite|citation|source|reference)\b.*\b(required|must|include)\b", scan_text_clean, re.IGNORECASE))
    if not has_citation_requirement:
        s3_score += 2
        s3_verifiability_note = "no citation/source requirement found in output template"
    else:
        s3_verifiability_note = "citation/source requirement present (healthy)"

    # --- Shadow 4: Golden-Allergy ---
    s4_hits = scan_text(scan_text_clean, S4_MARKERS)
    s4_score = sum(h["score"] for h in s4_hits)

    # Count step-by-step patterns
    step_count = len(re.findall(r"\bstep \d+:", scan_text_clean, re.IGNORECASE))
    if step_count > 5:
        s4_score += 3
        s4_step_note = f"{step_count} numbered steps (>5 → rigid script risk)"
    else:
        s4_step_note = f"{step_count} numbered steps"

    # Check for judgment permission
    has_judgment_permission = bool(re.search(r"\b(use your judgment|adapt|if .+ does not (match|fit))\b", scan_text_clean, re.IGNORECASE))
    if not has_judgment_permission:
        s4_score += 2
        s4_judgment_note = "no judgment/adaptation permission found"
    else:
        s4_judgment_note = "judgment/adaptation permission present (healthy)"

    # --- Shadow 5: Sinkhole of Indifference ---
    s5_hits = scan_text(scan_text_clean, S5_MARKERS)
    s5_score = sum(h["score"] for h in s5_hits)

    # Check description field for polarization (use original text for frontmatter)
    desc_match = re.search(r"^description:\s*(.+?)(?:\n(?!\s)|\n---|\Z)", skill_text, re.MULTILINE | re.DOTALL)
    if desc_match:
        desc = desc_match.group(1).strip()
        # Generic-capability phrasing: "X that does Y" vs polarized "X that does Y by doing Z"
        has_polarization = bool(re.search(r"\b(by|through|via|not|never|instead of|rather than|commits? to)\b", desc, re.IGNORECASE))
        if not has_polarization:
            s5_score += 3
            s5_desc_note = f"description appears generic: '{desc[:80]}...'"
        else:
            s5_desc_note = "description appears polarized (healthy)"
    else:
        s5_desc_note = "no description field found"

    # Check for explicit Choice vector
    has_choice_vector = bool(re.search(r"\b(choice vector|commitment|polariz)\b", scan_text_clean, re.IGNORECASE))
    if not has_choice_vector:
        s5_score += 2
        s5_choice_note = "no explicit Choice vector / commitment language"
    else:
        s5_choice_note = "explicit Choice vector language present (healthy)"

    # --- Verdicts ---
    def verdict(score, low_threshold=3, high_threshold=7):
        if score >= high_threshold:
            return "high"
        elif score >= low_threshold:
            return "moderate"
        else:
            return "low"

    report = {
        "skill_path": str(skill_path),
        "skill_md_lines": skill_lines,
        "reference_files": ref_files,
        "shadows": {
            "S1_dark_addiction": {
                "score": s1_score,
                "verdict": verdict(s1_score),
                "line_note": s1_line_note,
                "overlap_note": s1_overlap_note,
                "markers_hit": s1_hits[:10],  # cap for readability
                "markers_hit_count": len(s1_hits),
            },
            "S2_dark_allergy": {
                "score": s2_score,
                "verdict": verdict(s2_score),
                "example_note": s2_example_note,
                "why_note": s2_why_note,
                "markers_hit": s2_hits[:10],
                "markers_hit_count": len(s2_hits),
            },
            "S3_golden_addiction": {
                "score": s3_score,
                "verdict": verdict(s3_score),
                "verifiability_note": s3_verifiability_note,
                "markers_hit": s3_hits[:10],
                "markers_hit_count": len(s3_hits),
            },
            "S4_golden_allergy": {
                "score": s4_score,
                "verdict": verdict(s4_score),
                "step_note": s4_step_note,
                "judgment_note": s4_judgment_note,
                "markers_hit": s4_hits[:10],
                "markers_hit_count": len(s4_hits),
            },
            "S5_sinkhole_of_indifference": {
                "score": s5_score,
                "verdict": verdict(s5_score),
                "description_note": s5_desc_note,
                "choice_vector_note": s5_choice_note,
                "markers_hit": s5_hits[:10],
                "markers_hit_count": len(s5_hits),
            },
        },
        "recommended_focus": [],
    }

    # Recommend focus based on highest-scoring shadows
    shadow_scores = [
        ("S1_dark_addiction", s1_score),
        ("S2_dark_allergy", s2_score),
        ("S3_golden_addiction", s3_score),
        ("S4_golden_allergy", s4_score),
        ("S5_sinkhole_of_indifference", s5_score),
    ]
    shadow_scores.sort(key=lambda x: -x[1])
    for name, score in shadow_scores[:2]:
        if score >= 3:
            report["recommended_focus"].append({
                "shadow": name,
                "score": score,
                "verdict": verdict(score),
            })

    return report


def main():
    if len(sys.argv) < 2:
        print("Usage: python diagnose.py <path-to-skill-directory-or-SKILL.md>")
        sys.exit(1)

    skill_path = sys.argv[1]
    report = diagnose_skill(skill_path)

    if "error" in report:
        print(f"Error: {report['error']}", file=sys.stderr)
        sys.exit(1)

    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
