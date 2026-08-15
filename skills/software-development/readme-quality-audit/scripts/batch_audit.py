#!/usr/bin/env python3
"""Batch README audit across multiple repos."""

import json
import subprocess
import sys
import argparse
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

def fetch_readme(owner: str, repo: str, output_dir: Path) -> tuple[str, str | None]:
    """Fetch README from GitHub API and save to file."""
    output_file = output_dir / f"{repo}_readme.md"
    
    try:
        result = subprocess.run(
            ["gh", "api", f"repos/{owner}/{repo}/readme"],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode != 0:
            return repo, f"API error: {result.stderr}"
        
        import base64
        content = base64.b64decode(json.loads(result.stdout)["content"]).decode("utf-8")
        output_file.write_text(content)
        return repo, None
    except Exception as e:
        return repo, str(e)

def analyze_readme(readme_path: Path, script_path: Path) -> dict:
    """Run analyze_readme.py on a README file."""
    try:
        result = subprocess.run(
            [sys.executable, str(script_path), str(readme_path)],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode == 0:
            return json.loads(result.stdout)
        else:
            return {"repo": readme_path.stem.replace("_readme", ""), "error": result.stderr}
    except Exception as e:
        return {"repo": readme_path.stem.replace("_readme", ""), "error": str(e)}

def main():
    parser = argparse.ArgumentParser(description="Batch audit READMEs against readme-craft quality bar")
    parser.add_argument("--owner", required=True, help="GitHub owner/org")
    parser.add_argument("--repos", required=True, help="Comma-separated repo names")
    parser.add_argument("--output", default="/tmp/readme_audit", help="Output directory")
    parser.add_argument("--parallel", type=int, default=4, help="Parallel fetches")
    args = parser.parse_args()

    output_dir = Path(args.output)
    output_dir.mkdir(parents=True, exist_ok=True)
    
    repos = [r.strip() for r in args.repos.split(",")]
    script_path = Path(__file__).parent / "analyze_readme.py"
    
    if not script_path.exists():
        print(f"ERROR: analyze_readme.py not found at {script_path}", file=sys.stderr)
        sys.exit(1)

    print(f"Fetching {len(repos)} READMEs from {args.owner}...")
    
    # Fetch all READMEs in parallel
    fetch_results = {}
    with ThreadPoolExecutor(max_workers=args.parallel) as executor:
        futures = {executor.submit(fetch_readme, args.owner, repo, output_dir): repo for repo in repos}
        for future in as_completed(futures):
            repo, error = future.result()
            if error:
                print(f"  ❌ {repo}: {error}")
                fetch_results[repo] = {"error": error}
            else:
                print(f"  ✅ {repo}")
                fetch_results[repo] = {"fetched": True}

    # Analyze all READMEs
    print("\nAnalyzing READMEs...")
    analysis_results = {}
    for repo in repos:
        if repo not in fetch_results or "error" in fetch_results[repo]:
            continue
        readme_path = output_dir / f"{repo}_readme.md"
        if readme_path.exists():
            result = analyze_readme(readme_path, script_path)
            analysis_results[repo] = result
            score = result.get("score", 0)
            print(f"  {repo}: {score}/10")

    # Summary
    print("\n" + "=" * 60)
    print("BATCH AUDIT SUMMARY")
    print("=" * 60)
    
    ranked = sorted(
        [(r, analysis_results[r].get("score", 0)) for r in repos if r in analysis_results],
        key=lambda x: x[1], reverse=True
    )
    
    for i, (repo, score) in enumerate(ranked, 1):
        result = analysis_results[repo]
        issues = len(result.get("priority_fixes", []))
        print(f"  {i}. {repo}: {score}/10 ({issues} priority fixes)")

    # Save full results
    output_file = output_dir / "batch_audit_results.json"
    output_file.write_text(json.dumps({
        "owner": args.owner,
        "repos": repos,
        "results": analysis_results
    }, indent=2))
    print(f"\nFull results saved to {output_file}")

if __name__ == "__main__":
    main()