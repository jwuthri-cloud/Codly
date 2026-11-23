# gh answers for the intake tests

These files are written by hand. They are not recordings of a real `gh` run.

| File | What it stands for | Written from |
|---|---|---|
| `issue_view.json` | `gh issue view 12 --json number,title,body,state,url,author,labels,comments` | the fields `gh issue view --json` documents |
| `pr_view.json` | `gh pr view 7 --json number,title,body,state,url,author,labels,baseRefName,headRefName,isCrossRepository,comments,reviews` | the fields `gh pr view --json` documents |
| `pr_review_comments.json` | `gh api repos/acme/web/pulls/7/comments?…&direction=desc` (newest first) | GitHub's REST documentation, "List review comments on a pull request" |
| `pr_checks.json` | `gh pr checks <url> --json name,bucket` | the fields `gh pr checks --json` documents |

The repository `acme/web` and every person in them are made up. One issue comment tries to
close Codly's quoted block and to pass for the user: the tests check that it stays quoted.
