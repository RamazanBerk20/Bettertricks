import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { TooltipProvider } from "@radix-ui/react-tooltip";
import { vi } from "vitest";

import { App } from "../app";
import { ProgressBar } from "../components/common";
import { OperationDrawer, ReviewDialog } from "../components/dialogs";
import { Sidebar } from "../components/sidebar";
import { api } from "../lib/api";
import { ActivityView } from "../views/activity-view";
import { PrefixView } from "../views/prefix-view";
import type { OperationEvent, OperationPlan, OperationRecord, WinePrefix } from "../types";

describe("Bettertricks desktop shell", () => {
  it("boots into a discovered prefix and opens the searchable catalog", async () => {
    render(<TooltipProvider><App /></TooltipProvider>);

    expect(await screen.findByRole("heading", { name: "Default prefix" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Components/ }));
    expect(await screen.findByRole("heading", { name: "Components" })).toBeInTheDocument();

    const search = screen.getByPlaceholderText(/Search DLLs/);
    fireEvent.change(search, { target: { value: "win10" } });
    expect(await screen.findByText("Windows 10 compatibility mode")).toBeInTheDocument();
  });

  it("exposes navigation and selection state to assistive technology", async () => {
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    expect(screen.getByRole("navigation", { name: "Wine prefixes" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Skip to content" })).toHaveAttribute("href", "#main-content");
    expect(screen.getByRole("main")).toHaveAttribute("id", "main-content");
    expect(screen.getByRole("button", { name: /Overview/ })).toHaveAttribute("aria-current", "page");

    fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
    await screen.findByRole("heading", { name: "Settings" });
    expect(screen.getByRole("button", { name: "System" })).toHaveAttribute("aria-pressed", "true");
  });

  it("switches and persists the interface language without restarting", async () => {
    const saveSettings = vi.spyOn(api, "saveSettings");
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
    const language = await screen.findByRole("button", { name: "Interface language" });
    expect(language).toHaveTextContent("System");
    fireEvent.pointerDown(language, { button: 0, ctrlKey: false });
    fireEvent.click(await screen.findByRole("menuitemradio", { name: "Türkçe" }));

    expect(await screen.findByRole("heading", { name: "Ayarlar" })).toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute("lang", "tr");
    await waitFor(() => expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({ language: "tr" })));

    fireEvent.pointerDown(screen.getByRole("button", { name: "Arayüz dili" }), { button: 0, ctrlKey: false });
    fireEvent.click(await screen.findByRole("menuitemradio", { name: "English" }));
    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute("lang", "en");

    fireEvent.pointerDown(screen.getByRole("button", { name: "Interface language" }), { button: 0, ctrlKey: false });
    fireEvent.click(await screen.findByRole("menuitemradio", { name: "System" }));
    await waitFor(() => expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ language: "system" })));
    expect(screen.getByRole("button", { name: "Interface language" })).toHaveTextContent("System");
    vi.restoreAllMocks();
  });

  it("confirms before permanently clearing restore points", async () => {
    const clearRestorePoints = vi.spyOn(api, "clearRestorePoints").mockResolvedValueOnce({
      cleared: 1,
      protected: 0,
      restore_points: [],
    });
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
    const clear = await screen.findByRole("button", { name: "Clear restore points" });
    fireEvent.click(clear);

    const dialog = screen.getByRole("dialog", { name: "Clear restore points?" });
    expect(dialog).toHaveTextContent("permanently deleted");
    expect(dialog).toHaveTextContent("active operations");
    fireEvent.click(within(dialog).getByRole("button", { name: "Clear restore points" }));

    await waitFor(() => expect(clearRestorePoints).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Clear restore points?" })).not.toBeInTheDocument());
    clearRestorePoints.mockRestore();
  });

  it("runs command-palette results with arrow keys and Enter", async () => {
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Search or run/ }));
    const search = await screen.findByRole("combobox", { name: "Search commands and prefixes" });
    expect(search).toHaveAttribute("aria-activedescendant", "command-option-0");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    expect(search).toHaveAttribute("aria-activedescendant", "command-option-1");
    fireEvent.keyDown(search, { key: "Enter" });
    expect(await screen.findByRole("heading", { name: "Activity" })).toBeInTheDocument();
  });

  it("shows the local Steam game name instead of its app id", async () => {
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Baldur's Gate 3/ }));
    expect(await screen.findByRole("heading", { name: "Baldur's Gate 3" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "1086940" })).not.toBeInTheDocument();
  });

  it("reports progress as a bounded percentage", () => {
    render(<ProgressBar value={0.42} label="Installing" />);
    expect(screen.getByRole("progressbar", { name: "Installing" })).toHaveAttribute("aria-valuenow", "42");
  });

  it("keeps a hidden running operation available from the sidebar", async () => {
    const payload = await api.bootstrap();
    const reopen = vi.fn();
    render(
      <Sidebar
        view="catalog"
        onViewChange={() => undefined}
        prefixes={payload.prefixes}
        selectedPrefixId={payload.prefixes[0]?.id ?? null}
        onPrefixChange={() => undefined}
        onAddPrefix={() => undefined}
        onCommandPalette={() => undefined}
        catalog={payload.catalog}
        system={payload.system}
        runningCount={1}
        operationStatus={{ title: "Installing runtime", detail: "42% · 3 of 7", state: "running" }}
        onOpenOperation={reopen}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Show current operation: Installing runtime" }));
    expect(reopen).toHaveBeenCalledOnce();
  });

  it("collects required recipe information before an operation can start", async () => {
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Components/ }));
    const search = await screen.findByPlaceholderText(/Search DLLs/);
    fireEvent.change(search, { target: { value: "MIDIMap" } });
    await screen.findByText("Set MIDImap device");
    fireEvent.click(screen.getByRole("button", { name: "Add Set MIDImap device to selection" }));
    fireEvent.click(screen.getByRole("button", { name: /Review changes/ }));

    const input = await screen.findByRole("textbox", { name: /MIDI device/ });
    const apply = screen.getByRole("button", { name: /Apply changes/ });
    expect(screen.getByRole("region", { name: "Review details" })).toContainElement(input);
    expect(input).toHaveAttribute("aria-required", "true");
    expect(apply).toBeDisabled();
    fireEvent.change(input, { target: { value: "FluidSynth MIDI" } });
    expect(apply).toBeEnabled();
  });

  it("plans tracked recipes through the matching Winetricks host", async () => {
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Components/ }));
    const search = await screen.findByRole("textbox", { name: "Search components" });
    fireEvent.change(search, { target: { value: "vcrun2022" } });
    const add = await screen.findByRole("button", { name: /Add Visual C\+\+ 2015-2022 runtime to selection/ });
    expect(add).toBeEnabled();
    fireEvent.click(add);
    fireEvent.click(screen.getByRole("button", { name: /Review changes/ }));
    expect(await screen.findByText(/uses the Winetricks compatibility host/)).toBeInTheDocument();
    expect(screen.getAllByText("Run vcrun2022 through Winetricks")).not.toHaveLength(0);
  });

  it("selects and deselects every available filtered result", async () => {
    render(<TooltipProvider><App /></TooltipProvider>);

    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Components/ }));
    const search = await screen.findByRole("textbox", { name: "Search components" });
    fireEvent.change(search, { target: { value: "win11" } });

    const selectAll = await screen.findByRole("button", { name: "Select all 1 available results" });
    fireEvent.click(selectAll);
    expect(screen.getByText("1 selected")).toBeInTheDocument();

    expect(screen.getByRole("button", { name: "Deselect all 1 available results" })).toBeInTheDocument();
    const deselectAll = screen.getByRole("button", { name: "Deselect all selected components" });
    fireEvent.click(deselectAll);
    expect(screen.queryByText("1 selected")).not.toBeInTheDocument();
  });

  it("installs the checksum-verified compatibility host from a tracked recipe", async () => {
    const payload = await api.bootstrap();
    const unavailableSystem = {
      ...payload.system,
      dependencies: payload.system.dependencies.map((dependency) => dependency.id === "winetricks"
        ? { ...dependency, available: false, path: null, version: null }
        : dependency),
    };
    vi.spyOn(api, "bootstrap").mockResolvedValueOnce({ ...payload, system: unavailableSystem });
    const install = vi.spyOn(api, "installCompatibilityHost").mockResolvedValue({
      ...unavailableSystem,
      dependencies: unavailableSystem.dependencies.map((dependency) => dependency.id === "winetricks"
        ? { ...dependency, available: true, path: "/home/user/.local/share/bettertricks/compatibility-hosts/winetricks-20260125", version: "20260125" }
        : dependency),
    });

    render(<TooltipProvider><App /></TooltipProvider>);
    await screen.findByRole("heading", { name: "Default prefix" });
    fireEvent.click(screen.getByRole("button", { name: /Components/ }));
    fireEvent.change(await screen.findByRole("textbox", { name: "Search components" }), { target: { value: "vcrun2022" } });
    const installButton = await screen.findByRole("button", { name: /Install verified host/ });
    fireEvent.click(installButton);

    await waitFor(() => expect(install).toHaveBeenCalledOnce());
    expect(await screen.findByText(/Available through Winetricks 20260125/)).toBeInTheDocument();
    vi.restoreAllMocks();
  });

  it("imports and checksum-verifies required manual downloads from review", async () => {
    let imported = false;
    const prefix = {
      id: "manual-download-prefix",
      name: "Manual download prefix",
      path: "/tmp/manual-download-prefix",
      source: "manual",
      architecture: "wow64",
      runtime: "/usr/bin/wine",
      runtime_label: "Wine 11.13",
      managed: false,
      exists: true,
      installed_verbs: [],
      size_bytes: null,
      last_modified: null,
    } satisfies WinePrefix;
    const basePlan = {
      id: "manual-download-operation",
      prefix,
      requested_recipes: ["manual_test"],
      resolved_recipes: ["manual_test"],
      steps: [{ recipe_id: "manual_test", recipe_title: "Manual test", step_index: 0, label: "Import installer", destructive: false }],
      inputs: [],
      downloads: [{ recipe_id: "manual_test", file_id: "installer", filename: "installer.exe", urls: ["https://example.invalid/installer"], cached: false, manual: true }],
      conflicts: [],
      warnings: [],
      restore_recommended: false,
      estimated_download_bytes: null,
      options: { force: false, unattended: false, verify: true, no_clean: false, isolate: false, torify: false, country: null, create_restore_point: false },
    } satisfies OperationPlan;
    vi.spyOn(api, "planOperation").mockImplementation(async () => ({
      ...basePlan,
      downloads: basePlan.downloads.map((download) => ({ ...download, cached: imported })),
    }));
    vi.spyOn(api, "selectManualFile").mockResolvedValue("/tmp/installer.exe");
    const openUrl = vi.spyOn(api, "openUrl").mockResolvedValue(undefined);
    vi.spyOn(api, "importManualFile").mockImplementation(async () => {
      imported = true;
      return "/tmp/cache/installer.exe";
    });

    render(
      <ReviewDialog
        open
        onOpenChange={() => undefined}
        prefix={prefix}
        recipeIds={["manual_test"]}
        settings={{ theme: "system", language: "en", catalog_auto_update: true, restore_before_managed_changes: true, show_advanced: false, reduced_motion: false, custom_wine_binary: null }}
        onStarted={() => undefined}
      />,
    );

    const choose = await screen.findByRole("button", { name: /Choose file/ });
    fireEvent.click(screen.getByRole("button", { name: /Open source/ }));
    expect(openUrl).toHaveBeenCalledWith("https://example.invalid/installer");
    expect(screen.getByRole("button", { name: /Apply changes/ })).toBeDisabled();
    fireEvent.click(choose);
    expect(await screen.findByText("Cached; checksum verified before use")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Apply changes/ })).toBeEnabled();
    vi.restoreAllMocks();
  });

  it("renders and answers attended operation prompts", async () => {
    const respond = vi.fn().mockResolvedValue(undefined);
    const close = vi.fn();
    const plan = {
      id: "operation-id",
      requested_recipes: ["test"],
      resolved_recipes: ["test"],
      steps: [],
      inputs: [],
      downloads: [],
      conflicts: [],
      warnings: [],
      restore_recommended: false,
      estimated_download_bytes: null,
      options: { force: false, unattended: false, verify: true, no_clean: false, isolate: false, torify: false, country: null, create_restore_point: false },
      prefix: {
        id: "prefix-id",
        name: "Test prefix",
        path: "/tmp/test-prefix",
        source: "manual",
        architecture: "win64",
        runtime: null,
        runtime_label: null,
        managed: false,
        exists: true,
        installed_verbs: [],
        size_bytes: null,
        last_modified: null,
      },
    } as OperationPlan;
    const event = {
      operation_id: "operation-id",
      sequence: 1,
      state: "waiting_for_user",
      step: 1,
      total_steps: 1,
      recipe_id: "test",
      title: "Confirmation needed",
      detail: null,
      progress: 0.5,
      prompt: {
        id: "prompt-id",
        level: "confirmation",
        title: "Continue installation?",
        message: "The installer needs confirmation.",
        choices: [{ id: "continue", label: "Continue", destructive: false }],
      },
      log_line: null,
      failure: null,
      timestamp: new Date().toISOString(),
    } as OperationEvent;

    render(<OperationDrawer operationId="operation-id" plan={plan} events={[event]} onClose={close} onCancel={() => undefined} onRespond={respond} onRetry={() => undefined} />);
    expect(screen.getByRole("alertdialog", { name: "Continue installation?" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toHaveFocus();
    const hide = screen.getByRole("button", { name: "Hide activity drawer" });
    expect(hide).toBeEnabled();
    fireEvent.click(hide);
    expect(close).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(respond).toHaveBeenCalledWith("prompt-id", "continue");
  });

  it("shows detailed component failures and retries one or all from the live drawer", () => {
    const retry = vi.fn();
    const prefix = {
      id: "prefix-id",
      name: "Test prefix",
      path: "/tmp/test-prefix",
      source: "manual",
      architecture: "win64",
      runtime: null,
      runtime_label: null,
      managed: false,
      exists: true,
      installed_verbs: [],
      size_bytes: null,
      last_modified: null,
    } satisfies WinePrefix;
    const plan = {
      id: "operation-id",
      requested_recipes: ["broken_component"],
      resolved_recipes: ["broken_component"],
      steps: [{ recipe_id: "broken_component", recipe_title: "Broken component", step_index: 0, label: "Install component", destructive: true }],
      inputs: [],
      downloads: [],
      conflicts: [],
      warnings: [],
      restore_recommended: false,
      estimated_download_bytes: null,
      options: { force: false, unattended: false, verify: true, no_clean: false, isolate: false, torify: false, country: null, create_restore_point: false },
      prefix,
    } satisfies OperationPlan;
    const failure = {
      recipe_id: "broken_component",
      recipe_title: "Broken component",
      kind: "failed" as const,
      message: "Winetricks exited with code 42. Last output: checksum rejected.",
    };
    const timestamp = new Date().toISOString();
    const events = [
      {
        operation_id: plan.id,
        sequence: 1,
        state: "running",
        step: 1,
        total_steps: 1,
        recipe_id: failure.recipe_id,
        title: "Broken component failed",
        detail: failure.message,
        progress: 1,
        prompt: null,
        log_line: null,
        failure,
        timestamp,
      },
      {
        operation_id: plan.id,
        sequence: 2,
        state: "failed",
        step: 1,
        total_steps: 1,
        recipe_id: null,
        title: "Completed with failures",
        detail: "Finished the remaining jobs: 0 components succeeded and 1 component failed.",
        progress: 1,
        prompt: null,
        log_line: null,
        failure: null,
        timestamp,
      },
    ] satisfies OperationEvent[];

    render(<OperationDrawer operationId={plan.id} plan={plan} events={events} onClose={() => undefined} onCancel={() => undefined} onRespond={async () => undefined} onRetry={retry} />);

    expect(screen.getByText(/Last output: checksum rejected/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry Broken component" }));
    expect(retry).toHaveBeenLastCalledWith(["broken_component"]);
    fireEvent.click(screen.getByRole("button", { name: "Retry all (1)" }));
    expect(retry).toHaveBeenLastCalledWith(["broken_component"]);
  });

  it("keeps failure diagnostics and retry actions in activity history", () => {
    const retry = vi.fn();
    const operation = {
      id: "failed-operation",
      prefix_id: "failed-prefix",
      prefix_name: "Failed prefix",
      recipes: ["broken_component"],
      state: "failed",
      created_at: new Date().toISOString(),
      started_at: new Date().toISOString(),
      finished_at: new Date().toISOString(),
      current_step: 1,
      total_steps: 1,
      message: "Finished the remaining jobs with one failure.",
      failures: [{
        recipe_id: "broken_component",
        recipe_title: "Broken component",
        kind: "failed",
        message: "Installer returned code 42 after its checksum was rejected.",
      }],
    } satisfies OperationRecord;

    render(<ActivityView operations={[operation]} onRefresh={() => undefined} onClearActivity={async () => undefined} onRetryRecipes={retry} />);
    fireEvent.click(screen.getByRole("button", { name: "Show details for Failed prefix operation" }));

    expect(screen.getByText(/checksum was rejected/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry Broken component" }));
    expect(retry).toHaveBeenLastCalledWith("failed-prefix", ["broken_component"]);
    fireEvent.click(screen.getByRole("button", { name: "Retry all (1)" }));
    expect(retry).toHaveBeenLastCalledWith("failed-prefix", ["broken_component"]);
  });

  it("confirms before clearing finished activity", async () => {
    const clearActivity = vi.fn().mockResolvedValue(undefined);
    const operation = {
      id: "finished-operation",
      prefix_id: "finished-prefix",
      prefix_name: "Finished prefix",
      recipes: ["corefonts"],
      state: "succeeded",
      created_at: new Date().toISOString(),
      started_at: new Date().toISOString(),
      finished_at: new Date().toISOString(),
      current_step: 1,
      total_steps: 1,
      message: "Operation complete",
      failures: [],
    } satisfies OperationRecord;

    render(<ActivityView operations={[operation]} onRefresh={() => undefined} onClearActivity={clearActivity} onRetryRecipes={() => undefined} />);
    fireEvent.click(screen.getByRole("button", { name: "Clear activity" }));
    const dialog = screen.getByRole("dialog", { name: "Clear activity history?" });
    expect(dialog).toHaveTextContent("Active operations stay visible");
    fireEvent.click(within(dialog).getByRole("button", { name: "Clear activity" }));

    await waitFor(() => expect(clearActivity).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Clear activity history?" })).not.toBeInTheDocument());
  });

  it("offers safe installer launch and non-destructive forgetting for manual prefixes", async () => {
    const runInstaller = vi.fn();
    const forgetPrefix = vi.fn();
    const prefix = {
      id: "manual-prefix",
      name: "Manual test prefix",
      path: "/tmp/manual-prefix",
      source: "manual",
      architecture: "wow64",
      runtime: "/usr/bin/wine",
      runtime_label: "Wine 11.13",
      managed: false,
      exists: true,
      installed_verbs: [],
      size_bytes: null,
      last_modified: null,
    } satisfies WinePrefix;

    render(
      <PrefixView
        prefix={prefix}
        operations={[]}
        restorePoints={[]}
        onBrowseCatalog={() => undefined}
        onOpenPath={() => undefined}
        onLaunchTool={() => undefined}
        onCreateRestorePoint={() => undefined}
        onForgetPrefix={forgetPrefix}
        onRunInstaller={runInstaller}
        onTrashPrefix={() => undefined}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Run installer/ }));
    expect(runInstaller).toHaveBeenCalledOnce();

    fireEvent.pointerDown(screen.getByRole("button", { name: "More prefix actions" }), { button: 0, ctrlKey: false });
    const forget = await screen.findByRole("menuitem", { name: /Forget without deleting/ });
    fireEvent.click(forget);
    expect(forgetPrefix).toHaveBeenCalledOnce();
  });

  it("keeps the Overview installed-component summary bounded", () => {
    const prefix = {
      id: "large-prefix",
      name: "Large test prefix",
      path: "/tmp/large-prefix",
      source: "manual",
      architecture: "wow64",
      runtime: "/usr/bin/wine",
      runtime_label: "Wine 11.13",
      managed: false,
      exists: true,
      installed_verbs: Array.from({ length: 100 }, (_, index) => `verb_${index}`),
      size_bytes: null,
      last_modified: null,
    } satisfies WinePrefix;

    const { container } = render(
      <PrefixView
        prefix={prefix}
        operations={[]}
        restorePoints={[]}
        onBrowseCatalog={() => undefined}
        onOpenPath={() => undefined}
        onLaunchTool={() => undefined}
        onCreateRestorePoint={() => undefined}
        onForgetPrefix={() => undefined}
        onRunInstaller={() => undefined}
        onTrashPrefix={() => undefined}
      />,
    );

    expect(container.querySelectorAll(".installed-item")).toHaveLength(12);
    expect(screen.getByText("Showing the 12 most recently recorded of 100.")).toBeInTheDocument();
    expect(screen.getByText("verb_99")).toBeInTheDocument();
    expect(screen.queryByText("verb_0")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /View all 100/ })).toBeInTheDocument();
  });
});
