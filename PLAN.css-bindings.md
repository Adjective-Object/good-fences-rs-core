# css bindings

We want to build a system that uses our segment graph, and can:

- extract property:element assignments
- pass those into an optimizer to minimize the smallest set of classnames
  we need across a program to replicate the same set of properties

We're working on a simplified model, where:

- a component consists of a jsx/tsx file, and one or more associated css files:

```css
/* mycomponent.css */
.outer {
    width: 100%;
    height: 2em;
    background-color: red;
}

.inner {
    width: 100%;
    height: 2em;
    background-color: green
}
```

```tsx
// MyComponent.ts
import {outer, inner} from "./mycomponent.css"

function MyComponent() {
    return <div className={outer}>
        My Component Says Hi
        <div className={inner}>
            My inner div Says Hi
        </div>
    </div>
}
```

- css files are only a subset of CSS: root-level class declarations and property
  assignments. We do not allow e.g. pseudo-selectors, cascading descendents. This
  allows us to back out an assignment of element-ids to property names.

The above example would be optimized into something like:

```css
.a {
    width: 100%;
    height: 2em;
}

.b {
    background-color: red;
}

.c {
    background-color: green
}
```

```tsx
import {a, b, c} from "./mycomponent.css"

const MyComponent = () => {
    <div className={classnames(a, b)}>
        My Component Says Hi
        <div className={classnames(a, c)}>
            My inner div Says Hi
        </div>
    </div>
}
```

## Parse approach

- implement css parser to extract some JSON object of classname:property assignments
- implement TSX/JSX parser to extract component-classname bindings
  - include the source locations for replacing classnames
- assume global classnames & match on string names

`parse-css *.css > css-properties.json`
`parse-tsx *.tsx > tsx-class-bindings.json`

```json
{
    "classes": [
        {
            "name":"baz", 
            "properties": {
                "foo": "bar"
            }
        }
    ]
}
```

```json
{
    "file": "./MyComponent.tsx"
    "span": ":15:23..:15:40"
    "classNames": [
        "wrapper",
        "inner"
    ]
}
```

`optimize css-properties.json tsx-class-bindings.json -o optimized.json`

```json
{
    "css": ...,
    "elements": ...,
}
```

Demo on react to-do list
