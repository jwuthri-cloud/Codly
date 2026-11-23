"""D17: sandbox on, in a linked git worktree: can `git commit` write to the main repo's .git?

argv[1]: linked (cwd = a worktree made with `git worktree add`; its git dir is <main>/.git/worktrees/<name>)
         plain  (cwd = an ordinary repo, .git inside the cwd: control)
         linked-off (the linked worktree with no sandbox: control)
A request to run outside the sandbox is denied, so only the sandboxed path is recorded. The commit uses a demo identity,
no signing and no hooks (the user's global git hooks stay out). Both repos are throw-away folders under work/.
A second turn writes a file into the MAIN repo's working tree (outside the cwd of a linked worktree) to show whether
the sandbox is in force.
"""
import json, os, subprocess, sys
from drv import Session, make_repo
variant = sys.argv[1] if len(sys.argv) > 1 else "linked"
main = make_repo("r4-wt-main-" + variant)
cwd = main
if variant.startswith("linked"):
    cwd = os.path.join(os.path.dirname(main), "r4-wt-linked-" + variant)
    if not os.path.isdir(cwd):
        subprocess.run(["git", "worktree", "add", "-q", "-b", "wt-" + variant, cwd], cwd=main, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
seen = []

def handler(req):
    seen.append(req)
    inp = req.get("input", {})
    if inp.get("dangerouslyDisableSandbox"):
        return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
    return {"behavior": "allow", "updatedInput": inp}

def git(*a):
    return subprocess.run(["git"] + list(a), cwd=main, capture_output=True, text=True).stdout.strip()

extra = ["--tools", "Bash"]
if variant != "linked-off":
    extra += ["--settings", json.dumps({"sandbox": {"enabled": True}})]
s = Session("r4-sandbox-worktree-" + variant, extra=extra, hard_timeout=150, on_can_use_tool=handler, cwd=cwd)
before = git("rev-list", "--all", "--count")
s.note(git_dir=subprocess.run(["git", "rev-parse", "--git-dir"], cwd=cwd, capture_output=True, text=True).stdout.strip(),
       git_common_dir=subprocess.run(["git", "rev-parse", "--git-common-dir"], cwd=cwd, capture_output=True, text=True).stdout.strip(),
       commits_before=before)
s.control("get_sandbox_dialog")
s.user("Run exactly this Bash command once (do not retry it, do not change it) and report its output: echo wt > wt.txt && git add wt.txt && git -c core.hooksPath=/dev/null -c user.name=demo -c user.email=demo@example.invalid -c commit.gpgsign=false commit -m wt")
print("result:", s.wait_results(1, 100))
after = git("rev-list", "--all", "--count")
probe = os.path.join(main, "outside-probe.txt")
if variant.startswith("linked"):
    s.user("Run exactly this Bash command once (do not retry it, do not change it) and report its output: echo probe > ../%s/outside-probe.txt" % os.path.basename(main))
    print("result 2:", s.wait_results(2, 100))
unrelated = os.path.join(os.path.dirname(main), "r4-wt-unrelated-" + variant)
os.makedirs(unrelated, exist_ok=True)
if variant == "linked":
    # control: a folder that is neither the worktree nor its main repo
    s.user("Run exactly this Bash command once (do not retry it, do not change it) and report its output: echo probe > ../%s/outside-probe.txt" % os.path.basename(unrelated))
    print("result 3:", s.wait_results(3, 100))
s.note(outside_probe_written=os.path.exists(probe), unrelated_folder_probe_written=os.path.exists(os.path.join(unrelated, "outside-probe.txt")))
s.note(commits_after=after, commit_was_written=after != before, can_use_tool_requests=[(r["tool_name"], r.get("decision_reason_type"), bool(r.get("input", {}).get("dangerouslyDisableSandbox"))) for r in seen])
print("commits before/after:", before, after, "requests:", [(r["tool_name"], r.get("decision_reason_type")) for r in seen])
print("cost:", s.cost(), "exit:", s.close())
