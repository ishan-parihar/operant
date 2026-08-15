# Audit Rubric Implementation

## Scoring Function

```python
def score_readme(content: str) -> dict:
    """
    Returns: {strengths: list[str], issues: list[str], score: int}
    Target: 8 strengths, 0 issues, score = 8
    """
    lines = content.split("\n")
    total_lines = len(lines)
    total_chars = len(content)
    
    # 1. Value proposition (first paragraph)
    first_para = get_first_paragraph(content)
    value_prop = has_value_prop(first_para)
    
    # 2. Proof before architecture
    proof_before_arch = check_proof_before_arch(content)
    
    # 3. Visual proof depth
    images = count_images(content)
    visual_proof = images >= 3
    
    # 4. Install before architecture
    install_before_arch = check_install_before_arch(content)
    
    # 5. Code examples
    code_blocks = count_code_blocks(content)
    code_examples = code_blocks >= 2
    
    # 6. Structured tables
    tables = count_tables(content)
    has_tables = tables >= 1 and total_lines > 100
    
    # 7. Badges
    badges = count_badges(content)
    badges_ok = 3 <= badges <= 8
    
    # 8. Length
    length_ok = 100 <= total_lines <= 170
    
    checks = [
        ("value_prop_clear", value_prop),
        ("proof_before_arch", proof_before_arch),
        ("visual_proof_3plus", visual_proof),
        ("install_before_arch", install_before_arch),
        ("code_examples_2plus", code_examples),
        ("tables_for_structure", has_tables),
        ("badges_3_to_8", badges_ok),
        ("length_100_170", length_ok),
    ]
    
    strengths = [name for name, passed in checks if passed]
    issues = [name for name, passed in checks if not passed]
    score = len(strengths) - len(issues)
    
    return {"strengths": strengths, "issues": issues, "score": score}
```

## Helper Functions

```python
import re

def get_first_paragraph(content: str) -> str:
    for line in content.split("\n")[:30]:
        if line.strip() and not line.startswith("#") and not line.startswith("!") and not line.startswith("["):
            return line.strip().lower()
    return ""

def has_value_prop(para: str) -> bool:
    keywords = ["mcp", "tool", "server", "cli", "agent", "automation", 
                "video", "carousel", "intelligence", "trading", "deploy",
                "gateway", "orchestrat", "memory", "reddit", "social"]
    return any(k in para for k in keywords)

def check_proof_before_arch(content: str) -> bool:
    # Find "Visual proof" or "Proof" or "Screenshot" section index
    # Find "Architecture" section index
    # Return True if proof index < arch index
    proof_idx = find_section_index(content, ["visual proof", "proof", "screenshot", "output", "example", "showcase", "demo"])
    arch_idx = find_section_index(content, ["architecture", "tech stack", "project structure"])
    if proof_idx is not None and arch_idx is not None:
        return proof_idx < arch_idx
    return True  # No architecture section = pass

def find_section_index(content: str, keywords: list) -> int | None:
    lines = content.split("\n")
    for i, line in enumerate(lines):
        if line.startswith("## ") or line.startswith("### "):
            title = line.lstrip("# ").strip().lower()
            if any(k in title for k in keywords):
                return i
    return None

def count_images(content: str) -> int:
    return len(re.findall(r'!\[.*?\]\((.*?)\)', content))

def check_install_before_arch(content: str) -> bool:
    install_idx = find_section_index(content, ["install", "quick start", "getting started"])
    arch_idx = find_section_index(content, ["architecture", "tech stack", "project structure"])
    if install_idx is not None and arch_idx is not None:
        return install_idx < arch_idx
    return True

def count_code_blocks(content: str) -> int:
    return len(re.findall(r'```', content)) // 2

def count_tables(content: str) -> int:
    return len(re.findall(r'\|.*\|.*\n\|[-:| ]+\|', content))

def count_badges(content: str) -> int:
    return len(re.findall(r'!\[.*?\]\(https://img\.shields\.io', content))
```

## Usage in Audit Script

```bash
python3 -c "
import sys
sys.path.insert(0, '.')
from audit_rubric import score_readme
with open(sys.argv[1]) as f:
    result = score_readme(f.read())
print(f'Score: {result[\"score\"]}/8')
print(f'Strengths: {len(result[\"strengths\"])}')
print(f'Issues: {result[\"issues\"]}')
" README.md
```