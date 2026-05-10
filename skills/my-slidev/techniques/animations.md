# Animations: Code Explanation Patterns

Slidev provides a click-based animation system for progressive content reveal. This file focuses on **code explanation patterns** — the most impactful use of animations in technical presentations.

## Code Line Highlighting

Use `{line-ranges}` after the language identifier to step through code blocks click by click. Separate steps with `|`.

### Syntax

````md
```ts {all|2|1-6|all}
interface User {
  id: number
  firstName: string
  lastName: string
  role: string
}
```
````

| Step | Highlight | Effect |
|------|-----------|--------|
| Initial | `all` | Show full code, all lines highlighted |
| Click 1 | `2` | Only line 2 highlighted |
| Click 2 | `1-6` | Lines 1–6 highlighted |
| Click 3 | `all` | All lines highlighted again |

### Starting Hidden

Use `{hide|...}` to start with the code block hidden and reveal on first click:

````md
```ts {hide|all|2|3-4}
const a = 1
const b = 2
const c = a + b
```
````

## Progressive Reveal with v-click

Wrap elements in `v-click` to reveal them one click at a time.

### Single Element

```md
# Key Concept

Base content visible immediately.

<v-click>

- This appears on first click

</v-click>

<v-click>

- This appears on second click

</v-click>
```

### List Items

Wrap a list in `<v-clicks>` to reveal items one by one:

```md
# Benefits

<v-clicks>

- Faster build times
- Smaller bundle size
- Better developer experience

</v-clicks>
```

## Simultaneous Reveal with v-after

Use `v-after` to make an element appear at the same time as the previous `v-click`:

```md
<v-click>

**Step 1**: Define the interface

</v-click>
<v-after>

This explanation appears together with Step 1.

</v-after>
```

## Practical Patterns

### Pattern: Explain a Function Step by Step

````md
---
layout: blank
---

# How `updateUser` Works

```ts {all|2|3|4|all}
function updateUser(id: number, update: Partial<User>) {
  const user = getUser(id)          // Fetch current user
  const newUser = { ...user, ...update }  // Merge changes
  saveUser(id, newUser)             // Persist to database
}
```

<v-clicks>

- **Line 2**: Retrieve the existing user record
- **Line 3**: Spread operator merges only changed fields
- **Line 4**: Save the updated user back to storage

</v-clicks>
````

### Pattern: Before / After Comparison

````md
---
layout: blank
---

# Refactoring Example

<v-click>

**Before:**
```ts {all}
if (user !== null && user !== undefined) {
  return user.name
}
```

</v-click>

<v-click>

**After:**
```ts {all}
return user?.name
```

</v-click>
````

### Pattern: Highlight Key Lines Then Show Full Code

````md
---
layout: blank
---

# Error Handling Pattern

```ts {3-4|1-8|all}
async function fetchData(url: string) {
  try {
    const response = await fetch(url)
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    return await response.json()
  } catch (error) {
    console.error('Fetch failed:', error)
    return null
  }
}
```
````

## Tips

- Keep step counts reasonable (3–5 steps per code block) for audience attention
- Start with the most important line, not line 1
- Use `v-clicks` on bullet explanations that accompany code highlighting
- Combine code highlighting with `v-click` explanations for maximum clarity
