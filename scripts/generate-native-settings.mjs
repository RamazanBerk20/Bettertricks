import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { delimiter, join, resolve } from "node:path";
import {
  parseBuiltinDllOverrides,
  WINETRICKS_BASELINE,
  WINETRICKS_BUILTIN_DLL_COUNT,
  WINETRICKS_BUILTIN_DLL_SHA256,
} from "./catalog-metadata.mjs";

const root = resolve(import.meta.dirname, "..");
const output = join(root, "catalog", "native", "settings");
const upstreamTag = WINETRICKS_BASELINE;
const generatedMarker = "# Generated from audited Winetricks settings translations. Do not edit by hand.";
const winetricks = process.env.WINETRICKS ?? "winetricks";

const versionOutput = execFileSync(winetricks, ["--version"], { encoding: "utf8" });
const installedTag = versionOutput.match(/\b\d{8}\b/)?.[0] ?? "unknown";
if (installedTag !== upstreamTag) {
  throw new Error(`Native catalog generation requires Winetricks ${upstreamTag}; found ${installedTag}.`);
}
const winetricksSource = readFileSync(resolveExecutable(winetricks), "utf8");
const builtinDlls = parseBuiltinDllOverrides(winetricksSource);
const builtinDllHash = createHash("sha256").update(builtinDlls.join("\n")).digest("hex");
if (
  builtinDlls.length !== WINETRICKS_BUILTIN_DLL_COUNT
  || builtinDllHash !== WINETRICKS_BUILTIN_DLL_SHA256
) {
  throw new Error(
    `Builtin DLL list does not match audited Winetricks ${upstreamTag} `
      + `(${builtinDlls.length} entries, ${builtinDllHash}).`,
  );
}

mkdirSync(output, { recursive: true });
for (const filename of readdirSync(output)) {
  const path = join(output, filename);
  if (filename.endsWith(".toml") && readFileSync(path, "utf8").startsWith(generatedMarker)) {
    rmSync(path);
  }
}

const recipes = [];

const win32Only = new Set(["nt351", "nt40", "win20", "win2k", "win30", "win31", "win95", "win98", "winme"]);
for (const [id, title, version = id] of [
  ["nt351", "Windows NT 3.51"],
  ["nt40", "Windows NT 4.0"],
  ["vista", "Windows Vista"],
  ["win20", "Windows 2.0"],
  ["win2k", "Windows 2000"],
  ["win2k3", "Windows Server 2003"],
  ["win2k8", "Windows Server 2008"],
  ["win2k8r2", "Windows Server 2008 R2"],
  ["win30", "Windows 3.0"],
  ["win31", "Windows 3.1"],
  ["win7", "Windows 7"],
  ["win8", "Windows 8"],
  ["win81", "Windows 8.1"],
  ["win10", "Windows 10"],
  ["win11", "Windows 11"],
  ["win95", "Windows 95"],
  ["win98", "Windows 98"],
  ["winme", "Windows ME"],
  ["winver=", "Default Windows version (Windows 7)", "win7"],
  ["winxp", "Windows XP"],
]) {
  recipes.push({
    id,
    title: `Set Windows version to ${title}`,
    description: `Report ${title} to applications in the selected prefix.`,
    tags: ["windows version", "compatibility"],
    architectures: win32Only.has(id) ? ["win32"] : [],
    steps: [{ type: "windows_version", version }],
  });
}

for (const [id, title, smoothing, orientation, smoothingType] of [
  ["fontsmooth=disable", "Disable font smoothing", "0", "1", "0"],
  ["fontsmooth=gray", "Enable grayscale font smoothing", "2", "1", "1"],
  ["fontsmooth=bgr", "Enable BGR subpixel font smoothing", "2", "0", "2"],
  ["fontsmooth=rgb", "Enable RGB subpixel font smoothing", "2", "1", "2"],
]) {
  recipes.push({
    id,
    title,
    description: `${title} in the selected prefix.`,
    tags: ["fonts", "display"],
    steps: [{
      type: "native_action",
      action: "font_smoothing",
      parameters: { smoothing, orientation, smoothing_type: smoothingType },
    }],
  });
}

const stringSettings = [
  ["graphics=wayland", "Set graphics driver to Wayland", "HKEY_CURRENT_USER\\Software\\Wine\\Drivers", "Graphics", "wayland,x11"],
  ["graphics=x11", "Set graphics driver to X11", "HKEY_CURRENT_USER\\Software\\Wine\\Drivers", "Graphics", "x11"],
  ["graphics=mac", "Set graphics driver to Quartz (for macOS)", "HKEY_CURRENT_USER\\Software\\Wine\\Drivers", "Graphics", "mac,x11"],
  ["mwo=force", "Set DirectInput MouseWarpOverride to force", "HKEY_CURRENT_USER\\Software\\Wine\\DirectInput", "MouseWarpOverride", "force"],
  ["mwo=enabled", "Set DirectInput MouseWarpOverride to enabled", "HKEY_CURRENT_USER\\Software\\Wine\\DirectInput", "MouseWarpOverride", "enabled"],
  ["mwo=disable", "Set DirectInput MouseWarpOverride to disable", "HKEY_CURRENT_USER\\Software\\Wine\\DirectInput", "MouseWarpOverride", "disable"],
  ["grabfullscreen=y", "Force cursor clipping for full-screen windows", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "GrabFullscreen", "y"],
  ["grabfullscreen=n", "Disable cursor clipping for full-screen windows", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "GrabFullscreen", "n"],
  ["windowmanagerdecorated=y", "Allow the window manager to decorate windows", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "Decorated", "y"],
  ["windowmanagerdecorated=n", "Prevent the window manager from decorating windows", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "Decorated", "n"],
  ["windowmanagermanaged=y", "Allow the window manager to control windows", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "Managed", "y"],
  ["windowmanagermanaged=n", "Prevent the window manager from controlling windows", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "Managed", "n"],
  ["useegl=y", "Enable EGL", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "UseEGL", "Y"],
  ["useegl=n", "Disable EGL and use GLX", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "UseEGL", "N"],
  ["usetakefocus=y", "Enable UseTakeFocus", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "UseTakeFocus", "Y"],
  ["usetakefocus=n", "Disable UseTakeFocus", "HKEY_CURRENT_USER\\Software\\Wine\\X11 Driver", "UseTakeFocus", "N"],
  ["mimeassoc=on", "Enable desktop MIME associations", "HKEY_CURRENT_USER\\Software\\Wine\\FileOpenAssociations", "Enable", "Y"],
  ["mimeassoc=off", "Disable desktop MIME associations", "HKEY_CURRENT_USER\\Software\\Wine\\FileOpenAssociations", "Enable", "N"],
  ...["alsa", "coreaudio", "disabled", "oss", "pulse"].map((value) => [`sound=${value}`, `Set sound driver to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Drivers", "Audio", value]),
  ...["gdi", "gl", "no3d", "vulkan"].map((value) => [`renderer=${value}`, `Set renderer to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "renderer", value]),
  ["cfc=enabled", "Enable CheckFloatConstants", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "CheckFloatConstants", "enabled"],
  ["cfc=disabled", "Disable CheckFloatConstants", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "CheckFloatConstants", "disabled"],
  ...["0", "1", "2", "3"].map((value) => [`gsm=${value}`, `Set MaxShaderModelGS to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "MaxShaderModelGS", value]),
  ["npm=repack", "Set NonPower2Mode to repack", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "NonPower2Mode", "repack"],
  ...["backbuffer", "fbo"].map((value) => [`orm=${value}`, `Set OffscreenRenderingMode to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "OffscreenRenderingMode", value]),
  ...["0", "1", "2", "3"].map((value) => [`psm=${value}`, `Set MaxShaderModelPS to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "MaxShaderModelPS", value]),
  ...["arb", "glsl", "none"].map((value) => [`shader_backend=${value}`, `Set shader backend to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "shader_backend", value]),
  ...["auto", "disabled", "readdraw", "readtex", "texdraw", "textex"].map((value) => [`rtlm=${value}`, `Set RenderTargetLockMode to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "RenderTargetLockMode", value]),
  ...["0", "1", "2", "3"].map((value) => [`vsm=${value}`, `Set MaxShaderModelVS to ${value}`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "MaxShaderModelVS", value]),
  ...["512", "1024", "2048"].map((value) => [`videomemorysize=${value}`, `Report ${value} MB of video memory`, "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "VideoMemorySize", value]),
  ["hidewineexports=enable", "Enable hiding Wine exports", "HKEY_CURRENT_USER\\Software\\Wine", "HideWineExports", "Y"],
  ["autostart_winedbg=enabled", "Launch winedbg after unhandled exceptions", "HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows NT\\CurrentVersion\\AeDebug", "Debugger", "winedbg --auto %ld %ld"],
  ["autostart_winedbg=disabled", "Prevent winedbg after unhandled exceptions", "HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows NT\\CurrentVersion\\AeDebug", "Debugger", "false"],
];

for (const [id, title, key, name, value] of stringSettings) {
  recipes.push(registryRecipe(id, title, key, name, `"${value}"`));
}

for (const [id, title, key, name] of [
  ["graphics=default", "Set graphics driver to default", "HKEY_CURRENT_USER\\Software\\Wine\\Drivers", "Graphics"],
  ["videomemorysize=default", "Let Wine detect video memory", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "VideoMemorySize"],
  ["hidewineexports=disable", "Disable hiding Wine exports", "HKEY_CURRENT_USER\\Software\\Wine", "HideWineExports"],
]) {
  recipes.push(registryRecipe(id, title, key, name, "-"));
}

recipes.push({
  id: "alldlls=builtin",
  title: "Override most common DLLs to builtin",
  description: "Force Wine's builtin implementations for the 720 DLLs audited by upstream Winetricks.",
  tags: ["dll override", "compatibility"],
  steps: [{ type: "dll_override", mode: "builtin", libraries: builtinDlls }],
});

recipes.push({
  id: "alldlls=default",
  title: "Remove all DLL overrides",
  description: "Remove every global DLL override from the selected prefix.",
  tags: ["dll override", "reset"],
  steps: [{
    type: "registry",
    content: "[-HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides]",
    architecture: "prefix",
  }],
});

recipes.push({
  id: "hosts",
  title: "Add Windows hosts and services files",
  description: "Create missing Windows hosts and services files without replacing existing content.",
  tags: ["networking", "compatibility"],
  steps: [
    { type: "ensure_directory", path: "${system32}/drivers/etc" },
    { type: "ensure_file", path: "${system32}/drivers/etc/hosts" },
    { type: "ensure_file", path: "${system32}/drivers/etc/services" },
  ],
});

recipes.push({
  id: "isolate_home",
  title: "Remove wineprefix links to $HOME",
  description: "Replace Wine user-directory links into the host home directory with empty prefix-local directories.",
  tags: ["sandbox", "privacy", "filesystem"],
  steps: [{ type: "native_action", action: "isolate_home" }],
});

recipes.push({
  id: "sandbox",
  title: "Sandbox the wineprefix - remove links to HOME",
  description: "Remove the Z: mapping, disable Wine's Unix folder namespace, and isolate Wine user directories from HOME.",
  tags: ["sandbox", "privacy", "filesystem"],
  steps: [
    { type: "remove_symlink", path: "${prefix}/dosdevices/z:" },
    {
      type: "registry",
      content: "[-HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Desktop\\Namespace\\{9D20AAE8-0625-44B0-9CA7-71889C2254D9}]",
      architecture: "prefix",
    },
    { type: "native_action", action: "isolate_home" },
  ],
});

recipes.push({
  id: "fontfix",
  title: "Check for broken host fonts",
  description: "Detect the Samyak/Oriya X11 font known to crash some .NET applications.",
  tags: ["fonts", "diagnostics"],
  steps: [{ type: "native_action", action: "fontfix_check" }],
});

recipes.push({
  id: "bad",
  title: "Intentional failure test",
  description: "Fail intentionally to exercise operation error handling and recovery.",
  tags: ["test", "diagnostics"],
  steps: [{
    type: "native_action",
    action: "intentional_failure",
    parameters: { message: "The upstream bad verb failed intentionally." },
  }],
});

for (const [id, title, left, right] of [
  ["mackeyremap=both", "Enable Mac key remapping for both sides", "y", "y"],
  ["mackeyremap=left", "Enable Mac key remapping for the left side", "y", "n"],
  ["mackeyremap=none", "Disable Mac key remapping", "n", "n"],
]) {
  recipes.push({
    id,
    title,
    description: `${title} in the selected prefix.`,
    tags: ["keyboard", "macos"],
    steps: [{
      type: "registry",
      content: `[HKEY_CURRENT_USER\\Software\\Wine\\Mac Driver]\n"LeftCommandIsCtrl"="${left}"\n"LeftOptionIsAlt"="${left}"\n"RightCommandIsCtrl"="${right}"\n"RightOptionIsAlt"="${right}"`,
      architecture: "prefix",
    }],
  });
}

const themeColors = [
  ["ActiveBorder", "49 54 58"],
  ["ActiveTitle", "49 54 58"],
  ["AppWorkSpace", "60 64 72"],
  ["Background", "49 54 58"],
  ["ButtonAlternativeFace", "200 0 0"],
  ["ButtonDkShadow", "154 154 154"],
  ["ButtonFace", "49 54 58"],
  ["ButtonHilight", "119 126 140"],
  ["ButtonLight", "60 64 72"],
  ["ButtonShadow", "60 64 72"],
  ["ButtonText", "219 220 222"],
  ["GradientActiveTitle", "49 54 58"],
  ["GradientInactiveTitle", "49 54 58"],
  ["GrayText", "155 155 155"],
  ["Hilight", "119 126 140"],
  ["HilightText", "255 255 255"],
  ["InactiveBorder", "49 54 58"],
  ["InactiveTitle", "49 54 58"],
  ["InactiveTitleText", "219 220 222"],
  ["InfoText", "159 167 180"],
  ["InfoWindow", "49 54 58"],
  ["Menu", "49 54 58"],
  ["MenuBar", "49 54 58"],
  ["MenuHilight", "119 126 140"],
  ["MenuText", "219 220 222"],
  ["Scrollbar", "73 78 88"],
  ["TitleText", "219 220 222"],
  ["Window", "35 38 41"],
  ["WindowFrame", "49 54 58"],
  ["WindowText", "219 220 222"],
];

for (const [id, title, dark] of [
  ["theme=dark", "Use dark theme for the prefix", true],
  ["theme=light", "Use default Wine theme for the prefix", false],
]) {
  const colors = themeColors
    .map(([name, value]) => `"${name}"=${dark ? `"${value}"` : "-"}`)
    .join("\n");
  const themeManager = ["ColorName", "DllName", "LoadedBefore", "SizeName"]
    .map((name) => `"${name}"=-`)
    .concat(`"ThemeActive"=${dark ? '"0"' : "-"}`)
    .join("\n");
  const lightTheme = dark ? "00000000" : "00000001";
  recipes.push({
    id,
    title,
    description: `${title}.`,
    tags: ["theme", "appearance"],
    steps: [{
      type: "registry",
      content: `[HKEY_CURRENT_USER\\Control Panel\\Colors]\n${colors}\n\n[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\ThemeManager]\n${themeManager}\n\n[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize]\n"AppsUseLightTheme"=dword:${lightTheme}\n"SystemUsesLightTheme"=dword:${lightTheme}`,
      architecture: "prefix",
    }],
  });
}

for (const [id, title, key, name, value] of [
  ["csmt=off", "Disable command stream multithreading", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "csmt", "00000000"],
  ["csmt=on", "Enable command stream multithreading", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "csmt", "00000001"],
  ["csmt=force", "Force command stream serialization", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "csmt", "00000003"],
  ["ssm=disabled", "Disable strict shader math", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "strict_shader_math", "00000000"],
  ["ssm=enabled", "Enable strict shader math", "HKEY_CURRENT_USER\\Software\\Wine\\Direct3D", "strict_shader_math", "00000001"],
  ["heapcheck", "Enable heap checking with GlobalFlag", "HKEY_LOCAL_MACHINE\\System\\CurrentControlSet\\Control\\Session Manager", "GlobalFlag", "00200030"],
  ["nocrashdialog", "Disable Wine's crash dialog", "HKEY_CURRENT_USER\\Software\\Wine\\WineDbg", "ShowCrashDialog", "00000000"],
]) {
  recipes.push(registryRecipe(id, title, key, name, `dword:${value}`));
}

for (const [id, title, size] of [
  ["vd=off", "Disable virtual desktop", null],
  ["vd=640x480", "Enable a 640x480 virtual desktop", "640x480"],
  ["vd=800x600", "Enable an 800x600 virtual desktop", "800x600"],
  ["vd=1024x768", "Enable a 1024x768 virtual desktop", "1024x768"],
  ["vd=1280x1024", "Enable a 1280x1024 virtual desktop", "1280x1024"],
  ["vd=1440x900", "Enable a 1440x900 virtual desktop", "1440x900"],
]) {
  const value = size ? `"${size}"` : "-";
  const desktop = size ? '"Default"' : "-";
  recipes.push({
    id,
    title,
    description: `${title} in the selected prefix.`,
    tags: ["desktop", "display"],
    steps: [{ type: "registry", content: `[HKEY_CURRENT_USER\\Software\\Wine\\Explorer]\n"Desktop"=${desktop}\n\n[HKEY_CURRENT_USER\\Software\\Wine\\Explorer\\Desktops]\n"Default"=${value}`, architecture: "prefix" }],
  });
}

recipes.push({
  id: "forcemono",
  title: "Force Wine Mono instead of .NET",
  description: "Prefer Wine Mono for managed applications in the selected prefix.",
  tags: ["mono", "debugging"],
  steps: [
    { type: "dll_override", mode: "native", libraries: ["mscoree"] },
    { type: "dll_override", mode: "disabled", libraries: ["mscorsvw.exe"] },
  ],
});
recipes.push({
  id: "native_mdac",
  title: "Override odbc32, odbccp32 and oledb32",
  description: "Prefer native MDAC libraries with builtin fallbacks, including the upstream Wine 6.21 compatibility override.",
  tags: ["dll override", "database"],
  steps: [{ type: "native_action", action: "native_mdac" }],
});
recipes.push({
  id: "remove_mono",
  title: "Remove builtin wine-mono",
  description: "Uninstall all known Wine Mono package variants and remove their generated runtime shims.",
  tags: ["mono", "runtime", "cleanup"],
  steps: [{ type: "native_action", action: "remove_mono" }],
});
recipes.push({
  id: "set_mididevice",
  title: "Set MIDImap device",
  description: "Set the Windows MIDI mapper to a device selected in Bettertricks or supplied through MIDI_DEVICE.",
  tags: ["midi", "sound", "registry"],
  inputs: [{
    id: "device",
    label: "MIDI device",
    description: "The exact Windows MIDI output device name.",
    placeholder: "Microsoft GS Wavetable Synth",
    environment: "MIDI_DEVICE",
    required: true,
  }],
  steps: [{ type: "native_action", action: "set_midi_device" }],
});
recipes.push({
  id: "set_userpath",
  title: "Set the Wine user PATH",
  description: "Convert semicolon-separated native or Wine paths and store them as the prefix user's PATH.",
  tags: ["environment", "path", "registry"],
  inputs: [{
    id: "paths",
    label: "User PATH entries",
    description: "Native and/or Wine paths separated with semicolons. The WINEPATH environment variable is used when available.",
    placeholder: "/opt/tool/bin;C:\\Program Files\\Tool",
    environment: "WINEPATH",
    required: true,
  }],
  steps: [{ type: "native_action", action: "set_user_path" }],
});
recipes.push({
  id: "native_oleaut32",
  title: "Override oleaut32",
  description: "Prefer the native oleaut32 library with a builtin fallback.",
  tags: ["dll override"],
  steps: [{ type: "dll_override", mode: "native_builtin", libraries: ["oleaut32"] }],
});

const recipeIds = new Set();
for (const recipe of recipes) {
  if (recipeIds.has(recipe.id)) throw new Error(`Duplicate native recipe: ${recipe.id}`);
  recipeIds.add(recipe.id);
  writeRecipe(recipe);
}

process.stdout.write(`Generated ${recipes.length} native settings recipes.\n`);

function registryRecipe(id, title, key, name, encodedValue) {
  return {
    id,
    title,
    description: `${title} in the selected prefix.`,
    tags: ["registry", "compatibility"],
    steps: [{ type: "registry", content: `[${key}]\n"${name}"=${encodedValue}`, architecture: "prefix" }],
  };
}

function writeRecipe(recipe) {
  const lines = [
    generatedMarker,
    "schema = 1",
    `id = ${JSON.stringify(recipe.id)}`,
    'category = "settings"',
    `title = ${JSON.stringify(recipe.title)}`,
    `description = ${JSON.stringify(recipe.description)}`,
    'media = "none"',
    'maturity = "native"',
    `tags = [${recipe.tags.map((tag) => JSON.stringify(tag)).join(", ")}]`,
  ];
  if (recipe.architectures?.length) {
    lines.push("", "[constraints]", `architectures = [${recipe.architectures.map((value) => JSON.stringify(value)).join(", ")}]`);
  }
  for (const input of recipe.inputs ?? []) {
    lines.push("", "[[inputs]]", `id = ${JSON.stringify(input.id)}`, `label = ${JSON.stringify(input.label)}`);
    if (input.description) lines.push(`description = ${JSON.stringify(input.description)}`);
    if (input.placeholder) lines.push(`placeholder = ${JSON.stringify(input.placeholder)}`);
    if (input.environment) lines.push(`environment = ${JSON.stringify(input.environment)}`);
    lines.push(`required = ${Boolean(input.required)}`);
  }
  for (const step of recipe.steps) {
    lines.push("", "[[steps]]", `type = ${JSON.stringify(step.type)}`);
    for (const [key, value] of Object.entries(step)) {
      if (key === "type" || value === undefined) continue;
      if (key === "content") lines.push(`content = '''${value}'''`);
      else if (Array.isArray(value)) lines.push(`${key} = [${value.map((item) => JSON.stringify(item)).join(", ")}]`);
      else if (typeof value === "object") lines.push(`${key} = { ${Object.entries(value).map(([itemKey, itemValue]) => `${itemKey} = ${JSON.stringify(itemValue)}`).join(", ")} }`);
      else lines.push(`${key} = ${JSON.stringify(value)}`);
    }
  }
  lines.push("", "[source]", `upstream_tag = ${JSON.stringify(upstreamTag)}`, `upstream_verb = ${JSON.stringify(recipe.id)}`, "");
  writeFileSync(join(output, `${recipe.id}.toml`), lines.join("\n"));
}

function resolveExecutable(command) {
  const candidates = command.includes("/")
    ? [resolve(command)]
    : (process.env.PATH ?? "").split(delimiter).map((directory) => join(directory, command));
  const executable = candidates.find((candidate) => existsSync(candidate));
  if (!executable) throw new Error(`Cannot locate Winetricks executable: ${command}`);
  return executable;
}
