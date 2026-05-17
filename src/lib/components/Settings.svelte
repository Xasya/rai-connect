<script lang="ts">
  import { store, updateConfig, detectOsuPath, validateOsuPath, isConnected, createDesktopShortcut, checkShortcutExists, removeDesktopShortcut, disconnect, startProxy } from "$lib/stores/app.svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { Info, ExternalLink, Trash2 } from "lucide-svelte";
  import Button from "./Button.svelte";
  import Checkbox from "./Checkbox.svelte";
  import Tooltip from "./Tooltip.svelte";
  import { onMount } from "svelte";

  let isDetecting = $state(false);
  let pathInput = $state(store.config.osu_path ?? "");
  let shortcutExists = $state<boolean | null>(null);
  let isShortcutLoading = $state(false);
  let isServerRestarting = $state(false);

  const serverOptions = [
    { label: "Bancho", value: "ppy.sh" },
    { label: "gatari.pw", value: "gatari.pw" },
    { label: "ripple.moe", value: "ripple.moe" },
    { label: "akatsuki.gg", value: "akatsuki.gg" },
  ];

  const currentUpstream = store.config.proxy.upstream_server || "ppy.sh";
  const isKnownServer = serverOptions.some(o => o.value === currentUpstream);

  let serverSelectValue = $state(isKnownServer ? currentUpstream : "custom");
  let customServerInput = $state(currentUpstream);
  let showSupporterConfirm = $state(false);


  $effect(() => {
    const upstream = store.config.proxy.upstream_server || "ppy.sh";
    const known = serverOptions.some(o => o.value === upstream);
    
    if (known) {
      serverSelectValue = upstream;
    } else {
      serverSelectValue = "custom";
      customServerInput = upstream;
    }
  });

  onMount(() => {
    refreshShortcutStatus();
  });

  async function refreshShortcutStatus() {
    shortcutExists = await checkShortcutExists();
  }

  async function handleShortcutToggle() {
    isShortcutLoading = true;
    try {
      if (shortcutExists) {
        await removeDesktopShortcut();
        shortcutExists = false;
      } else {
        const result = await createDesktopShortcut();
        if (result) shortcutExists = true;
      }
    } finally {
      isShortcutLoading = false;
    }
  }

  async function handleDetect() {
    isDetecting = true;
    try {
      const detected = await detectOsuPath();
      if (detected) {
        pathInput = detected;
        await updateConfig("osu_path", detected);
      }
    } finally {
      isDetecting = false;
    }
  }

  async function handlePathChange() {
    if (!pathInput) return;
    const isValid = await validateOsuPath(pathInput);
    if (isValid) {
      await updateConfig("osu_path", pathInput);
    }
  }

  async function handleToggle(key: "start_at_boot" | "minimize_to_tray" | "start_minimized" | "debug_logging") {
    await updateConfig(key, !store.config[key]);
  }

  function normalizeServer(server: string): string {
    return server
      .trim()
      .replace(/^https?:\/\//i, "")
      .replace(/\/.*$/, "")
      .toLowerCase();
  }

  async function updateProxyConfig(nextProxy: typeof store.config.proxy, restartIfConnected = false) {
    const shouldRestart = restartIfConnected && isConnected();
    try {
      if (shouldRestart) {
        isServerRestarting = true;
        await disconnect();
      }

      await updateConfig("proxy", nextProxy);

      if (shouldRestart) {
        await startProxy();
      }
    } finally {
      isServerRestarting = false;
    }
  }

  async function handleServerSelect(event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value;
    serverSelectValue = value;

    if (value === "custom") {
      customServerInput = serverOptions.some((o) => o.value === store.config.proxy.upstream_server)
        ? ""
        : store.config.proxy.upstream_server;
      return;
    }

    customServerInput = value;
    await updateProxyConfig({ ...store.config.proxy, upstream_server: value }, true);
  }

  async function handleCustomServerChange() {
    const upstream = normalizeServer(customServerInput);
    customServerInput = upstream;

    if (!upstream || upstream === store.config.proxy.upstream_server) return;
    await updateProxyConfig({ ...store.config.proxy, upstream_server: upstream }, true);
  }

  async function handleSupporterCheckboxChange() {
    if (store.config.proxy.inject_supporter) {
      await updateProxyConfig({ ...store.config.proxy, inject_supporter: false });
    } else {
      showSupporterConfirm = true;
    }
  }

  async function confirmSupporter() {
    await updateProxyConfig({ ...store.config.proxy, inject_supporter: true });
    showSupporterConfirm = false;
  }

  async function cancelSupporter() {
    await openUrl("https://osu.ppy.sh/home/support");
    showSupporterConfirm = false;
  }
</script>

{#if showSupporterConfirm}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-sm p-4">
    <div class="bg-card border border-border rounded-xl p-6 max-w-md shadow-2xl">
      <h3 class="text-xl font-bold text-foreground mb-4">Wait a moment!</h3>
      <p class="text-muted-foreground mb-4 leading-relaxed">
        While we provide this feature for convenience, <strong class="text-foreground">osu! relies on supporter tags to survive</strong> and pay for servers.
      </p>
      <p class="text-muted-foreground mb-6 leading-relaxed">
        If you have the means, please consider buying a supporter tag to help keep the game alive.
      </p>
      <div class="flex gap-3 justify-end">
        <Button variant="outline" onclick={cancelSupporter}>
          {#snippet children()}
            I'll Support osu!
          {/snippet}
        </Button>
        <Button variant="destructive" onclick={confirmSupporter}>
          {#snippet children()}
            Enable Anyway
          {/snippet}
        </Button>
      </div>
    </div>
  </div>
{/if}

<div class="space-y-6">
  <div class="space-y-2">
    <div class="flex items-center gap-2">
      <label for="osu-path" class="block text-sm font-medium text-foreground">
        osu! Installation Path
      </label>
      <Tooltip text="The folder containing osu!.exe. Usually located at C:\osu! or in your Program Files." position="right">
        {#snippet children()}
          <Info class="w-4 h-4 text-muted-foreground cursor-help hover:text-foreground transition-colors" />
        {/snippet}
      </Tooltip>
    </div>
    <div class="flex flex-wrap gap-2">
      <input
        id="osu-path"
        type="text"
        bind:value={pathInput}
        onblur={handlePathChange}
        placeholder="C:\osu!"
        class="min-w-0 flex-1 basis-48 px-3 py-2 bg-input border border-input rounded-md text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring transition-all"
      />
      <Tooltip text="Automatically find your osu! installation" position="top">
        {#snippet children()}
          <Button variant="outline" onclick={handleDetect} loading={isDetecting}>
            {#snippet children()}Detect{/snippet}
          </Button>
        {/snippet}
      </Tooltip>
    </div>
    {#if store.config.osu_path}
      <p class="text-xs text-success">Valid osu! installation found</p>
    {:else if pathInput}
      <p class="text-xs text-destructive">osu! not found at this path</p>
    {:else}
      <p class="text-xs text-muted-foreground">Enter the path to your osu! folder or click Detect</p>
    {/if}
  </div>

  <div class="space-y-3">
    <label class="flex items-center gap-3 cursor-pointer group">
      <Checkbox
        checked={store.config.start_at_boot}
        onchange={() => handleToggle("start_at_boot")}
      />
      <div class="flex min-w-0 flex-col">
        <span class="text-sm text-foreground group-hover:text-primary transition-colors">
          Start at system boot
        </span>
        <span class="text-xs text-muted-foreground">
          Launch rai!connect when Windows starts
        </span>
      </div>
    </label>

    <label class="flex items-center gap-3 cursor-pointer group">
      <Checkbox
        checked={store.config.minimize_to_tray}
        onchange={() => handleToggle("minimize_to_tray")}
      />
      <div class="flex min-w-0 flex-col">
        <span class="text-sm text-foreground group-hover:text-primary transition-colors">
          Minimize to tray on close
        </span>
        <span class="text-xs text-muted-foreground">
          Keep the app running in background when closing the window
        </span>
      </div>
    </label>

    <label class="flex items-center gap-3 cursor-pointer group">
      <Checkbox
        checked={store.config.start_minimized}
        onchange={() => handleToggle("start_minimized")}
      />
      <div class="flex min-w-0 flex-col">
        <span class="text-sm text-foreground group-hover:text-primary transition-colors">
          Start minimized
        </span>
        <span class="text-xs text-muted-foreground">
          Hide the window when the app launches
        </span>
      </div>
    </label>

    <label class="flex items-center gap-3 cursor-pointer group">
      <Checkbox
        checked={store.config.debug_logging}
        onchange={() => handleToggle("debug_logging")}
      />
      <div class="flex min-w-0 flex-col">
        <span class="text-sm text-foreground group-hover:text-primary transition-colors">
          Show debug logs
        </span>
        <span class="text-xs text-muted-foreground">
          View internal logs for troubleshooting
        </span>
      </div>
    </label>

    <div class="pt-4 border-t border-border">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex min-w-0 flex-col">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium text-foreground">
              Desktop Shortcut
            </span>
            <Tooltip text="Creates a shortcut on your desktop that launches osu! through rai!connect in one click" position="right">
              {#snippet children()}
                <Info class="w-4 h-4 text-muted-foreground cursor-help hover:text-foreground transition-colors" />
              {/snippet}
            </Tooltip>
          </div>
          <span class="text-xs text-muted-foreground">
            {#if shortcutExists === null}
              Checking shortcut status...
            {:else if shortcutExists}
              Shortcut exists on desktop
            {:else}
              Launch osu! with rai in one click
            {/if}
          </span>
        </div>
        <Button
          variant={shortcutExists ? "destructive" : "outline"}
          onclick={handleShortcutToggle}
          loading={isShortcutLoading || shortcutExists === null}
          disabled={shortcutExists === null}
        >
          {#snippet children()}
            {#if shortcutExists}
              <Trash2 class="w-4 h-4 mr-1" />
              Remove
            {:else}
              <ExternalLink class="w-4 h-4 mr-1" />
              Create
            {/if}
          {/snippet}
        </Button>
      </div>
    </div>

    <div class="pt-4 border-t border-border">
      <label class="flex items-start gap-3 cursor-pointer group">
        <Checkbox
          checked={store.config.proxy.inject_supporter}
          onchange={handleSupporterCheckboxChange}
          class="mt-0.5"
        />
        <div class="flex min-w-0 flex-1 flex-col">
          <div class="flex items-center gap-2">
            <span class="text-sm text-foreground group-hover:text-primary transition-colors">
              Inject Supporter Tag
            </span>
            <Tooltip text="This modifies the API response to show supporter status. It does not actually give you osu!supporter features on the official servers." position="right">
              {#snippet children()}
                <Info class="w-4 h-4 text-muted-foreground cursor-help hover:text-foreground transition-colors" />
              {/snippet}
            </Tooltip>
          </div>
          <span class="text-xs text-muted-foreground">
            Shows the supporter heart icon in-game without requiring osu!supporter. This is purely cosmetic and unlocks osu!direct.
          </span>
        </div>
      </label>
    </div>

    <div class="pt-4 border-t border-border space-y-2">
      <div class="flex items-center gap-2">
        <label for="server-select" class="block text-sm font-medium text-foreground">
          Server
        </label>
        <Tooltip text="Select where Bancho traffic is forwarded. Beatmap downloads still use rai.moe, and osu! updates stay on official servers." position="right">
          {#snippet children()}
            <Info class="w-4 h-4 text-muted-foreground cursor-help hover:text-foreground transition-colors" />
          {/snippet}
        </Tooltip>
      </div>

      <div class="flex flex-wrap gap-2">
        <select
          id="server-select"
          value={serverSelectValue}
          onchange={handleServerSelect}
          disabled={isServerRestarting}
          class="min-w-0 w-full sm:w-48 px-3 py-2 bg-input border border-input rounded-md text-foreground focus:outline-none focus:ring-1 focus:ring-ring transition-all"
        >
          {#each serverOptions as option}
            <option value={option.value}>{option.label}</option>
          {/each}
          <option value="custom">Custom...</option>
        </select>

        {#if serverSelectValue === "custom"}
          <input
            id="custom-server"
            type="text"
            bind:value={customServerInput}
            onblur={handleCustomServerChange}
            onkeydown={(event) => {
              if (event.key === "Enter") {
                (event.currentTarget as HTMLInputElement).blur();
              }
            }}
            disabled={isServerRestarting}
            placeholder="ppy.sh"
            class="min-w-0 flex-1 basis-48 px-3 py-2 bg-input border border-input rounded-md text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring transition-all"
          />
          <Button
            variant="outline"
            class="shrink-0 h-full"
            onclick={handleCustomServerChange}
            disabled={isServerRestarting}
          >
            {#snippet children()}Save{/snippet}
          </Button>
        {/if}
      </div>

      <p class="text-xs text-muted-foreground">
        {#if isServerRestarting}
          Restarting proxy with {store.config.proxy.upstream_server}...
        {:else if store.config.proxy.upstream_server === "ppy.sh"}
          Traffic uses the official osu! servers.
        {:else}
          Traffic forwards to {store.config.proxy.upstream_server}.
        {/if}
      </p>
    </div>

    <!-- {#if isConnected()}
      <div class="p-3 bg-warning/10 border border-warning/20 rounded-lg flex items-center gap-2">
        <Info class="w-4 h-4 text-warning" />
        <p class="text-xs text-warning">
          Reconnect for changes to take effect.
        </p>
      </div>
    {/if} -->
  </div>
</div>
