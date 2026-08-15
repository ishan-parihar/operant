#!/usr/bin/env python3
"""Full quality bar scoring for READMEs against readme-craft criteria."""

from __future__ import annotations
import re
import sys
import json
from pathlib import Path
from dataclasses import dataclass, asdict
from typing import List, Dict, Any


@dataclass
class DimensionScore:
    score: float
    max: float
    issues: List[str]


@dataclass
class ReadmeAudit:
    repo: str
    score: float
    max_score: float
    dimensions: Dict[str, DimensionScore]
    priority_fixes: List[str]


MARKDOWN_IMAGE = re.compile(r"!\[[^\]]*\]\(([^)\s]+)(?:\s+[^)]*)?\)")
HTML_IMAGE = re.compile(r"<img\b[^>]*\bsrc=[\"']([^\"']+)[\"'][^>]*>", re.I)
HTML_ALT = re.compile(r"\balt=[\"']([^\"']*)[\"']", re.I)
HEADER = re.compile(r"^(#{1,3})\s+(.+)$", re.M)
CODE_BLOCK = re.compile(r"```[\s\S]*?```")
TABLE = re.compile(r"\|.*\|.*\n\|[-:| ]+\|")
BADGE = re.compile(r"!\[.*?\]\(https://img\.shields\.io/[^)]+\)")
EMOJI = re.compile(r"[\U0001F300-\U0001F9FF]")


def analyze_readme(content: str, repo: str = "unknown") -> ReadmeAudit:
    lines = content.split("\n")
    
    # --- Extract structural elements ---
    sections = []
    for m in HEADER.finditer(content):
        level = len(m.group(1))
        title = m.group(2).strip()
        sections.append((level, title, m.start()))
    
    # --- First paragraph (value prop) ---
    first_para = ""
    for line in lines[:30]:
        stripped = line.strip()
        if stripped and not stripped.startswith(("#", "!", "[", "`", "|", ">", "-", "*")):
            first_para = stripped[:500]
            break
    
    # --- Visual proof ---
    images = MARKDOWN_IMAGE.findall(content)
    html_tags = re.findall(r"<img\b[^>]*>", content, re.I)
    images.extend(HTML_IMAGE.findall(content))
    local_images = [i for i in images if not i.startswith(("http", "data:", "#"))]
    remote_images = [i for i in images if i.startswith("http")]
    
    # --- Code blocks ---
    code_blocks = CODE_BLOCK.findall(content)
    
    # --- Tables ---
    tables = TABLE.findall(content)
    
    # --- Badges ---
    badges = BADGE.findall(content)
    
    # --- Emoji count ---
    emoji_count = len(EMOJI.findall(content))
    
    # --- Length ---
    total_lines = len(lines)
    
    # --- Section ordering ---
    section_order = []
    for level, title, pos in sections:
        if level <= 2:
            t = title.lower()
            if any(k in t for k in ["install", "quick start", "getting started", "setup"]):
                section_order.append(("install", pos))
            elif any(k in t for k in ["architect", "tech stack", "project structure", "design"]):
                section_order.append(("architecture", pos))
            elif any(k in t for k in ["feature", "what it does", "capability", "key feature"]):
                section_order.append(("features", pos))
            elif any(k in t for k in ["demo", "screenshot", "output", "example", "showcase", "proof"]):
                section_order.append(("proof", pos))
            elif any(k in t for k in ["usage", "how to use", "cli", "api", "configuration"]):
                section_order.append(("usage", pos))
    
    # --- Scoring ---
    
    # 1. Hero/Value (max 2.0)
    hero_score = 0.0
    hero_issues = []
    if first_para:
        value_keywords = ["mcp", "tool", "server", "cli", "agent", "automation", "video", "carousel", 
                          "intelligence", "trading", "deploy", "framework", "platform", "engine", "gateway"]
        if any(k in first_para.lower() for k in value_keywords):
            hero_score += 1.0
        else:
            hero_issues.append("First paragraph lacks clear value proposition")
    else:
        hero_issues.append("No clear first paragraph found")
    
    # Hero visual proof
    hero_region = content[:500]
    if MARKDOWN_IMAGE.search(hero_region) or HTML_IMAGE.search(hero_region):
        hero_score += 1.0
    else:
        hero_issues.append("Hero lacks visual proof (no image/screenshot in first 500 chars)")
    
    # 2. Proof First (max 2.0)
    proof_score = 0.0
    proof_issues = []
    proof_positions = [pos for typ, pos in section_order if typ == "proof"]
    arch_positions = [pos for typ, pos in section_order if typ == "architecture"]
    feat_positions = [pos for typ, pos in section_order if typ == "features"]
    
    if proof_positions and arch_positions:
        if min(proof_positions) < min(arch_positions):
            proof_score += 2.0
        else:
            proof_score += 0.5
            proof_issues.append("Architecture appears before proof/visual evidence")
    elif proof_positions:
        proof_score += 2.0
    elif arch_positions:
        proof_score += 0.0
        proof_issues.append("Architecture section present but no proof section found")
    else:
        proof_score += 1.0  # neutral if neither
    
    # 3. Structure Order (max 1.5)
    struct_score = 0.0
    struct_issues = []
    install_positions = [pos for typ, pos in section_order if typ == "install"]
    usage_positions = [pos for typ, pos in section_order if typ == "usage"]
    
    if install_positions and arch_positions:
        if min(install_positions) < min(arch_positions):
            struct_score += 1.0
        else:
            struct_issues.append("Install/Quick Start comes after Architecture")
    elif install_positions:
        struct_score += 1.0
    else:
        struct_issues.append("No Install/Quick Start section found")
    
    if usage_positions and feat_positions:
        if min(usage_positions) > min(feat_positions):
            struct_score += 0.5
        else:
            struct_issues.append("Usage appears before Features")
    else:
        struct_score += 0.5
    
    # 4. Visual Proof (max 1.5)
    visual_score = 0.0
    visual_issues = []
    total_images = len(local_images) + len(remote_images)
    if total_images >= 3:
        visual_score = 1.5
    elif total_images >= 2:
        visual_score = 1.0
    elif total_images >= 1:
        visual_score = 0.5
        visual_issues.append(f"Only {total_images} image(s) - need 3+ for strong visual proof")
    else:
        visual_issues.append("Zero images - no visual proof of output/capability")
    
    # 5. Code Examples (max 1.0)
    code_score = 0.0
    code_issues = []
    if len(code_blocks) >= 2:
        code_score = 1.0
        # Check for end-to-end example
        e2e_keywords = ["install", "run", "build", "deploy", "script", "main", "example"]
        has_e2e = any(any(k in b.lower() for k in e2e_keywords) for b in code_blocks)
        if not has_e2e:
            code_issues.append("Code blocks present but no clear end-to-end example")
    elif len(code_blocks) == 1:
        code_score = 0.5
        code_issues.append("Only 1 code block - add end-to-end example")
    else:
        code_issues.append("No code examples - add install + usage examples")
    
    # 6. Tables/Structured Data (max 1.0)
    table_score = 0.0
    table_issues = []
    if len(tables) >= 2:
        table_score = 1.0
    elif len(tables) == 1:
        table_score = 0.5
        table_issues.append("Only 1 table - add feature/config/comparison tables")
    else:
        if total_lines > 100:
            table_issues.append("No tables for structured data (features, tools, config)")
    
    # 7. Length/Density (max 0.5)
    length_score = 0.5
    length_issues = []
    if total_lines > 500:
        length_score = 0.0
        length_issues.append(f"Very long README ({total_lines} lines) - condense to <300")
    elif total_lines > 400:
        length_score = 0.25
        length_issues.append(f"Long README ({total_lines} lines) - target <300")
    elif total_lines > 300:
        length_score = 0.5
        length_issues.append(f"Above ideal length ({total_lines} lines) - consider condensing")
    
    # 8. Badges/Signals (max 0.5)
    badge_score = 0.0
    badge_issues = []
    badge_count = len(badges)
    if 3 <= badge_count <= 8:
        badge_score = 0.5
    elif badge_count == 0:
        badge_issues.append("No badges - add 3-8 for quick signal (license, language, status)")
    elif badge_count > 10:
        badge_issues.append(f"Too many badges ({badge_count}) - reduce to 8 max")
        badge_score = 0.25
    else:
        badge_score = 0.5
        badge_issues.append(f"Low badge count ({badge_count}) - aim for 3-8")
    
    # Emoji check
    if emoji_count > 15:
        hero_issues.append(f"High emoji count ({emoji_count}) - consider reducing")
    
    # --- Compile dimensions ---
    dimensions = {
        "hero_value": DimensionScore(hero_score, 2.0, hero_issues),
        "proof_first": DimensionScore(proof_score, 2.0, proof_issues),
        "structure_order": DimensionScore(struct_score, 1.5, struct_issues),
        "visual_proof": DimensionScore(visual_score, 1.5, visual_issues),
        "code_examples": DimensionScore(code_score, 1.0, code_issues),
        "tables": DimensionScore(table_score, 1.0, table_issues),
        "length": DimensionScore(length_score, 0.5, length_issues),
        "badges": DimensionScore(badge_score, 0.5, badge_issues),
    }
    
    total_score = sum(d.score for d in dimensions.values())
    max_score = sum(d.max for d in dimensions.values())
    
    # --- Priority fixes ---
    all_issues = []
    for dim_name, dim in dimensions.items():
        for issue in dim.issues:
            all_issues.append(f"[{dim_name}] {issue}")
    
    # Sort by dimension weight (heuristic)
    priority_order = ["visual_proof", "proof_first", "hero_value", "structure_order", 
                      "code_examples", "tables", "length", "badges"]
    priority_fixes = []
    for dim_name in priority_order:
        for issue in dimensions[dim_name].issues:
            priority_fixes.append(f"[{dim_name}] {issue}")
    
    return ReadmeAudit(
        repo=repo,
        score=total_score,
        max_score=max_score,
        dimensions=dimensions,
        priority_fixes=priority_fixes[:8]  # Top 8
    )


def main():
    if len(sys.argv) != 2:
        print("usage: analyze_readme.py /path/to/README.md", file=sys.stderr)
        sys.exit(2)
    
    path = Path(sys.argv[1]).expanduser().resolve()
    if not path.is_file():
        print(f"ERROR: README not found: {path}", file=sys.stderr)
        sys.exit(2)
    
    content = path.read_text(encoding="utf-8")
    repo = path.parent.name
    audit = analyze_readme(content, repo)
    
    # Convert to dict for JSON output
    out = {
        "repo": audit.repo,
        "score": round(audit.score, 2),
        "max_score": audit.max_score,
        "dimensions": {k: asdict(v) for k, v in audit.dimensions.items()},
        "priority_fixes": audit.priority_fixes
    }
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()