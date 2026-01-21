---
description: Commit all changes without Claude attribution
argument-hint: Optional commit message
---

# Commit Changes

Commit all staged and unstaged changes to git without adding Claude co-author attribution.

## Instructions

1. Run `git status` and `git diff --stat` to see what will be committed
2. Run `git log --oneline -3` to see recent commit message style
3. If the user provided a message in $ARGUMENTS, use that as the commit message
4. Otherwise, analyze the changes and write a concise, descriptive commit message following conventional commits format (feat:, fix:, refactor:, docs:, test:, chore:)
5. Stage all changes with `git add -A`
6. Commit with the message - do NOT include any Co-Authored-By line
7. Show the resulting commit with `git show --stat HEAD`
