# Tailwind CSS Animation Patterns

## CSS Keyframe Animations

### Defining Custom Animations

```css
@theme {
  /* Animation definitions */
  --animate-fade-in: fade-in 0.3s ease-out;
  --animate-fade-out: fade-out 0.3s ease-out;
  --animate-slide-up: slide-up 0.4s ease-out;
  --animate-slide-down: slide-down 0.4s ease-out;
  --animate-scale-in: scale-in 0.2s ease-out;
  --animate-spin-slow: spin 3s linear infinite;
}

@keyframes fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}

@keyframes fade-out {
  from { opacity: 1; }
  to { opacity: 0; }
}

@keyframes slide-up {
  from {
    opacity: 0;
    transform: translateY(10px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes slide-down {
  from {
    opacity: 0;
    transform: translateY(-10px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes scale-in {
  from {
    opacity: 0;
    transform: scale(0.95);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}
```

### Usage

```html
<div class="animate-fade-in">Fades in</div>
<div class="animate-slide-up">Slides up</div>
<div class="animate-scale-in">Scales in</div>
```

## Transition Utilities

### Transition Properties

```html
<!-- Specific properties (recommended) -->
<button class="transition-colors duration-200">Color transition</button>
<button class="transition-transform duration-200">Transform transition</button>
<button class="transition-opacity duration-200">Opacity transition</button>
<button class="transition-shadow duration-200">Shadow transition</button>

<!-- Multiple properties -->
<button class="transition-[color,transform] duration-200">Custom</button>

<!-- All properties (use sparingly - performance impact) -->
<button class="transition-all duration-200">All properties</button>
```

### Duration Scale

```html
<div class="transition duration-75">75ms</div>
<div class="transition duration-100">100ms</div>
<div class="transition duration-150">150ms (default)</div>
<div class="transition duration-200">200ms</div>
<div class="transition duration-300">300ms</div>
<div class="transition duration-500">500ms</div>
<div class="transition duration-700">700ms</div>
<div class="transition duration-1000">1000ms</div>
```

### Timing Functions

```html
<div class="transition ease-linear">Linear</div>
<div class="transition ease-in">Ease in</div>
<div class="transition ease-out">Ease out (recommended)</div>
<div class="transition ease-in-out">Ease in-out</div>

<!-- Custom cubic-bezier -->
<div class="transition ease-[cubic-bezier(0.68,-0.55,0.265,1.55)]">
  Bouncy
</div>
```

### Delay

```html
<div class="transition delay-75">75ms delay</div>
<div class="transition delay-100">100ms delay</div>
<div class="transition delay-150">150ms delay</div>
<div class="transition delay-200">200ms delay</div>
<div class="transition delay-300">300ms delay</div>
```

## Interactive Animations

### Hover Effects

```html
<!-- Scale on hover -->
<div class="transition-transform duration-200 hover:scale-105">
  Grows on hover
</div>

<!-- Lift effect -->
<div class="transition-all duration-200 hover:-translate-y-1 hover:shadow-lg">
  Lifts on hover
</div>

<!-- Color shift -->
<button class="
  bg-blue-600 text-white
  transition-colors duration-200
  hover:bg-blue-700
">
  Button
</button>

<!-- Multi-property -->
<div class="
  transition-all duration-300
  hover:scale-105 hover:shadow-xl hover:bg-gray-50
">
  Complex hover
</div>
```

### Focus Effects

```html
<input class="
  border border-gray-300
  transition-all duration-200
  focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20
  focus:outline-none
" />

<button class="
  transition-all duration-200
  focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-2
">
  Accessible focus
</button>
```

### Active States

```html
<button class="
  transition-transform duration-100
  active:scale-95
">
  Press effect
</button>
```

## Group Animations

### Group Hover

```html
<div class="group cursor-pointer p-4 border rounded-lg hover:border-blue-500">
  <h3 class="transition-colors group-hover:text-blue-600">Title</h3>
  <p class="transition-colors group-hover:text-gray-600">Description</p>
  <span class="
    inline-block transition-transform
    group-hover:translate-x-1
  ">
    → Read more
  </span>
</div>
```

### Nested Groups

```html
<div class="group/card p-4 border rounded-lg">
  <div class="group/image relative overflow-hidden">
    <img class="
      transition-transform duration-300
      group-hover/image:scale-110
    " />
    <div class="
      absolute inset-0 bg-black/50
      opacity-0 transition-opacity
      group-hover/image:opacity-100
    ">
      Overlay
    </div>
  </div>
  <h3 class="transition-colors group-hover/card:text-blue-600">
    Title
  </h3>
</div>
```

## Staggered Animations

### CSS Custom Properties

```html
<div class="space-y-2">
  <div class="animate-slide-up" style="animation-delay: 0ms">Item 1</div>
  <div class="animate-slide-up" style="animation-delay: 100ms">Item 2</div>
  <div class="animate-slide-up" style="animation-delay: 200ms">Item 3</div>
  <div class="animate-slide-up" style="animation-delay: 300ms">Item 4</div>
</div>
```

### React Pattern

```tsx
{items.map((item, index) => (
  <div
    key={item.id}
    className="animate-slide-up opacity-0"
    style={{
      animationDelay: `${index * 100}ms`,
      animationFillMode: 'forwards',
    }}
  >
    {item.content}
  </div>
))}
```

## Loading Animations

### Spinner

```html
<svg class="animate-spin h-5 w-5" viewBox="0 0 24 24">
  <circle
    class="opacity-25"
    cx="12" cy="12" r="10"
    stroke="currentColor"
    stroke-width="4"
    fill="none"
  />
  <path
    class="opacity-75"
    fill="currentColor"
    d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
  />
</svg>
```

### Pulse

```html
<div class="animate-pulse flex space-x-4">
  <div class="rounded-full bg-gray-300 h-10 w-10"></div>
  <div class="flex-1 space-y-4 py-1">
    <div class="h-4 bg-gray-300 rounded w-3/4"></div>
    <div class="h-4 bg-gray-300 rounded"></div>
  </div>
</div>
```

### Skeleton

```css
@utility skeleton {
  background: linear-gradient(
    90deg,
    var(--color-gray-200) 25%,
    var(--color-gray-300) 50%,
    var(--color-gray-200) 75%
  );
  background-size: 200% 100%;
  animation: shimmer 1.5s infinite;
}

@keyframes shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
```

```html
<div class="skeleton h-4 w-full rounded"></div>
```

### Bounce Dots

```html
<div class="flex space-x-1">
  <div class="w-2 h-2 bg-blue-500 rounded-full animate-bounce" style="animation-delay: 0ms"></div>
  <div class="w-2 h-2 bg-blue-500 rounded-full animate-bounce" style="animation-delay: 150ms"></div>
  <div class="w-2 h-2 bg-blue-500 rounded-full animate-bounce" style="animation-delay: 300ms"></div>
</div>
```

## Accessibility

### Reduced Motion

```html
<!-- Respect user preferences -->
<div class="
  transition-transform duration-300
  motion-safe:hover:scale-105
  motion-reduce:transition-none
  motion-reduce:hover:scale-100
">
  Respects motion preferences
</div>
```

### CSS Approach

```css
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

## Performance Tips

### GPU-Accelerated Properties

Prefer these for smooth 60fps animations:
- `transform` (translate, scale, rotate)
- `opacity`

Avoid animating:
- `width`, `height`
- `top`, `left`, `right`, `bottom`
- `margin`, `padding`
- `border-width`

### will-change

```html
<!-- Use sparingly for complex animations -->
<div class="will-change-transform hover:scale-105">
  Optimized for transform
</div>
```

### contain

```html
<!-- Isolate repaints -->
<div class="contain-layout">
  Animation won't affect siblings
</div>
```

## Common Animation Recipes

### Modal Entrance

```html
<div class="
  fixed inset-0 bg-black/50
  animate-fade-in
">
  <div class="
    bg-white rounded-xl p-6
    animate-scale-in
  ">
    Modal content
  </div>
</div>
```

### Toast Notification

```html
<div class="
  fixed bottom-4 right-4
  animate-slide-up
  bg-gray-900 text-white px-4 py-3 rounded-lg shadow-lg
">
  Notification message
</div>
```

### Menu Dropdown

```html
<div class="
  absolute top-full left-0 mt-2
  origin-top-left
  animate-scale-in
  bg-white rounded-lg shadow-xl
">
  Menu items
</div>
```
