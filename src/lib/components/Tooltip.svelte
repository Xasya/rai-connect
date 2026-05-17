<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    text: string;
    position?: "top" | "bottom" | "left" | "right";
    delay?: number;
    children: Snippet;
  }

  let {
    text,
    position = "top",
    delay = 200,
    children,
  }: Props = $props();

  let showTooltip = $state(false);
  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  let triggerEl = $state<HTMLDivElement>();
  let tooltipEl = $state<HTMLDivElement>();
  let tooltipStyle = $state("");
  let arrowStyle = $state("");

  function updatePosition() {
    if (!triggerEl || !tooltipEl) return;

    const margin = 8;
    const trigger = triggerEl.getBoundingClientRect();
    const tooltip = tooltipEl.getBoundingClientRect();

    let top = 0;
    let left = 0;

    if (position === "bottom") {
      top = trigger.bottom + margin;
      left = trigger.left + trigger.width / 2 - tooltip.width / 2;
    } else if (position === "left") {
      top = trigger.top + trigger.height / 2 - tooltip.height / 2;
      left = trigger.left - tooltip.width - margin;
    } else if (position === "right") {
      top = trigger.top + trigger.height / 2 - tooltip.height / 2;
      left = trigger.right + margin;
    } else {
      top = trigger.top - tooltip.height - margin;
      left = trigger.left + trigger.width / 2 - tooltip.width / 2;
    }

    const maxLeft = window.innerWidth - tooltip.width - margin;
    const maxTop = window.innerHeight - tooltip.height - margin;
    left = Math.min(Math.max(margin, left), Math.max(margin, maxLeft));
    top = Math.min(Math.max(margin, top), Math.max(margin, maxTop));

    const arrowLeft = trigger.left + trigger.width / 2 - left;
    const arrowTop = trigger.top + trigger.height / 2 - top;

    tooltipStyle = `top: ${top}px; left: ${left}px;`;
    arrowStyle = position === "left" || position === "right"
      ? `top: ${Math.min(Math.max(8, arrowTop), tooltip.height - 8)}px;`
      : `left: ${Math.min(Math.max(8, arrowLeft), tooltip.width - 8)}px;`;
  }

  function handleMouseEnter() {
    timeoutId = setTimeout(() => {
      showTooltip = true;
      requestAnimationFrame(updatePosition);
    }, delay);
  }

  function handleMouseLeave() {
    if (timeoutId) {
      clearTimeout(timeoutId);
      timeoutId = null;
    }
    showTooltip = false;
  }

  function handleFocus() {
    showTooltip = true;
    requestAnimationFrame(updatePosition);
  }

  function handleBlur() {
    showTooltip = false;
  }

  const arrowClasses = $derived({
    top: "top-full -translate-x-1/2 border-t-popover border-x-transparent border-b-transparent",
    bottom: "bottom-full -translate-x-1/2 border-b-popover border-x-transparent border-t-transparent",
    left: "left-full -translate-y-1/2 border-l-popover border-y-transparent border-r-transparent",
    right: "right-full -translate-y-1/2 border-r-popover border-y-transparent border-l-transparent",
  }[position]);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="relative inline-flex"
  bind:this={triggerEl}
  onmouseenter={handleMouseEnter}
  onmouseleave={handleMouseLeave}
  onfocusin={handleFocus}
  onfocusout={handleBlur}
>
  {@render children()}

  {#if showTooltip}
    <div
      bind:this={tooltipEl}
      class="fixed z-50 pointer-events-none max-w-[calc(100vw-1rem)]"
      style={tooltipStyle}
      role="tooltip"
    >
      <div class="bg-popover text-popover-foreground text-xs px-3 py-2 rounded-md shadow-md border border-border max-w-xs whitespace-normal">
        {text}
      </div>
      <div class="absolute {arrowClasses} border-4 w-0 h-0" style={arrowStyle}></div>
    </div>
  {/if}
</div>
