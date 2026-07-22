import { memo, useCallback, useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import {
  ArrowRight,
  Check,
  CheckCircle,
  CloudArrowDown,
  Funnel,
  HardDrive,
  MagnifyingGlass,
  Package,
  Plus,
  ShieldCheck,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import { api } from "../lib/api";
import { titleCase } from "../lib/format";
import { useI18n, type TranslationValues } from "../lib/i18n";
import type {
  CatalogSummary,
  CatalogQuery,
  Recipe,
  RecipeListItem,
  UUID,
  VerbCategory,
  WinePrefix,
} from "../types";
import { Badge, Button, CategoryIcon, EmptyState, MaturityBadge, Skeleton } from "../components/common";

interface CatalogViewProps {
  prefix: WinePrefix | null;
  catalog: CatalogSummary;
  compatibilityHostReady: boolean;
  compatibilityHostInstalling: boolean;
  selectedRecipes: Set<string>;
  onToggleRecipe: (id: string) => void;
  onSetRecipeSelection: (ids: string[], selected: boolean) => void;
  onInstallCompatibilityHost: () => void;
  onReview: () => void;
  initialSearch?: string;
}

const categories: Array<VerbCategory | "all"> = ["all", "dlls", "fonts", "apps", "settings", "benchmarks"];

export function CatalogView({
  prefix,
  catalog,
  compatibilityHostReady,
  compatibilityHostInstalling,
  selectedRecipes,
  onToggleRecipe,
  onSetRecipeSelection,
  onInstallCompatibilityHost,
  onReview,
  initialSearch = "",
}: CatalogViewProps) {
  const { t, formatNumber } = useI18n();
  const [search, setSearch] = useState(initialSearch);
  const [category, setCategory] = useState<VerbCategory | "all">("all");
  const [filters, setFilters] = useState({ installed: false, cached: false, compatible: true });
  const [results, setResults] = useState<RecipeListItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [recipe, setRecipe] = useState<Recipe | null>(null);
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    const timeout = window.setTimeout(() => {
      const query: CatalogQuery = {
        search: search || null,
        category: category === "all" ? null : category,
        media: null,
        installed_only: filters.installed,
        cached_only: filters.cached,
        compatible_only: filters.compatible,
        prefix_id: prefix?.id ?? null,
      };
      setLoading(true);
      api.searchCatalog(query)
        .then((items) => {
          if (!current) return;
          setResults(items);
          setSelectedId((current) => {
            if (current && items.some((item) => item.id === current)) return current;
            return items.find((item) => item.maturity === "native")?.id ?? items[0]?.id ?? null;
          });
          setError(null);
        })
        .catch((reason) => {
          if (current) setError(String(reason));
        })
        .finally(() => {
          if (current) setLoading(false);
        });
    }, 140);
    return () => {
      current = false;
      window.clearTimeout(timeout);
    };
  }, [search, category, filters, prefix?.id]);

  useEffect(() => {
    let current = true;
    if (!selectedId) {
      setRecipe(null);
      return () => { current = false; };
    }
    setDetailLoading(true);
    api.getRecipe(selectedId)
      .then((nextRecipe) => {
        if (current) setRecipe(nextRecipe);
      })
      .catch((reason) => {
        if (current) setError(String(reason));
      })
      .finally(() => {
        if (current) setDetailLoading(false);
      });
    return () => { current = false; };
  }, [selectedId]);

  const selectRecipe = useCallback((id: string) => setSelectedId(id), []);

  const selectedCount = selectedRecipes.size;
  const selectableResultIds = useMemo(() => results.filter((item) => (
    Boolean(prefix)
    && item.compatible
    && (item.maturity === "native" || (item.maturity === "metadata_only" && compatibilityHostReady))
  )).map((item) => item.id), [results, prefix, compatibilityHostReady]);
  const allResultsSelected = selectableResultIds.length > 0 && selectableResultIds.every((id) => selectedRecipes.has(id));
  const activeFilterCount = Object.values(filters).filter(Boolean).length - (filters.compatible ? 1 : 0);
  const resultSummary = useMemo(() => {
    if (loading) return t("Searching catalog");
    return t("Results: {count}", { count: formatNumber(results.length) });
  }, [formatNumber, loading, results.length, t]);

  return (
    <div className="catalog-page">
      <header className="catalog-header">
        <div>
          <div className="heading-meta">
            <Badge tone="accent">{t("Catalog {version}", { version: catalog.upstream_tag })}</Badge>
            <span>{t("Winetricks-compatible entries: {count}", { count: formatNumber(catalog.recipe_count) })}</span>
          </div>
          <h1>{t("Components")}</h1>
          <p>{t("Find runtimes, fonts, applications, and compatibility settings for {prefix}.", { prefix: prefix?.name ?? t("a Wine prefix") })}</p>
        </div>
        <div className="catalog-prefix-chip">
          <HardDrive size={18} weight="duotone" />
          <span><small>{t("Installing into")}</small><strong>{prefix?.name ?? t("No prefix selected")}</strong></span>
        </div>
      </header>

      <div className="catalog-toolbar">
        <label className="search-field">
          <MagnifyingGlass size={18} />
          <input
            aria-label={t("Search components")}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t("Search DLLs, runtimes, fonts, settings…")}
            autoComplete="off"
          />
          {search ? <button onClick={() => setSearch("")} aria-label={t("Clear search")}><X size={15} /></button> : <kbd>/</kbd>}
        </label>
        <div className="filter-group" role="group" aria-label={t("Catalog filters")}>
          <FilterButton active={filters.installed} onClick={() => setFilters((value) => ({ ...value, installed: !value.installed }))}>{t("Installed")}</FilterButton>
          <FilterButton active={filters.cached} onClick={() => setFilters((value) => ({ ...value, cached: !value.cached }))}>{t("Cached")}</FilterButton>
          <FilterButton active={filters.compatible} onClick={() => setFilters((value) => ({ ...value, compatible: !value.compatible }))}>{t("Compatible")}</FilterButton>
          <span className="filter-icon-button" role="status" aria-label={t("Active filters: {count}", { count: formatNumber(activeFilterCount) })}>
            <Funnel size={17} />
            {activeFilterCount > 0 ? <span>{formatNumber(activeFilterCount)}</span> : null}
          </span>
        </div>
      </div>

      <div className="category-tabs" role="group" aria-label={t("Component categories")}>
        {categories.map((item) => (
          <button
            aria-pressed={category === item}
            className={clsx(category === item && "active")}
            key={item}
            onClick={() => setCategory(item)}
          >
            {categoryLabel(item, t)}
            {item !== "all" && catalog.categories[item] ? <small>{formatNumber(catalog.categories[item])}</small> : null}
          </button>
        ))}
      </div>

      <div className="catalog-workspace">
        <section className="catalog-list-panel" aria-label={t("Catalog results")}>
          <div className="result-count">
            <span>{resultSummary}</span>
            <div className="result-count-actions">
              <span>{t("Sorted by relevance")}</span>
              <button
                type="button"
                disabled={loading || selectableResultIds.length === 0}
                onClick={() => onSetRecipeSelection(selectableResultIds, !allResultsSelected)}
                aria-label={allResultsSelected ? t("Deselect all {count} available results", { count: formatNumber(selectableResultIds.length) }) : t("Select all {count} available results", { count: formatNumber(selectableResultIds.length) })}
              >
                {allResultsSelected ? t("Deselect all") : t("Select all")}
              </button>
            </div>
          </div>
          <div className="catalog-list">
            {loading ? (
              Array.from({ length: 8 }).map((_, index) => <CatalogSkeleton key={index} />)
            ) : error ? (
              <EmptyState icon={<WarningCircle size={24} />} title={t("Catalog unavailable")} body={error} />
            ) : results.length === 0 ? (
              <EmptyState
                icon={<MagnifyingGlass size={24} />}
                title={t("No matching components")}
                body={t("Try a broader search or remove one of the active filters.")}
                action={<Button size="small" onClick={() => { setSearch(""); setFilters({ installed: false, cached: false, compatible: true }); }}>{t("Reset filters")}</Button>}
              />
            ) : (
              results.map((item) => (
                <CatalogRow
                  key={item.id}
                  item={item}
                  active={selectedId === item.id}
                  queued={selectedRecipes.has(item.id)}
                  compatibilityHostReady={compatibilityHostReady}
                  hasPrefix={Boolean(prefix)}
                  onSelect={selectRecipe}
                  onToggle={onToggleRecipe}
                />
              ))
            )}
          </div>
        </section>

        <section className="recipe-detail-panel" aria-live="polite">
          {detailLoading ? <RecipeDetailSkeleton /> : recipe ? (
            <RecipeDetail
              recipe={recipe}
              item={results.find((item) => item.id === recipe.id)}
              queued={selectedRecipes.has(recipe.id)}
              prefix={prefix}
              compatibilityHostReady={compatibilityHostReady}
              compatibilityHostInstalling={compatibilityHostInstalling}
              onInstallCompatibilityHost={onInstallCompatibilityHost}
              onToggle={() => onToggleRecipe(recipe.id)}
            />
          ) : (
            <EmptyState icon={<Package size={26} />} title={t("Choose a component")} body={t("Select an entry to review its source, requirements, and expected changes.")} />
          )}
        </section>
      </div>

      {selectedCount > 0 ? (
        <div className="selection-tray">
          <div className="selection-stack" aria-hidden="true">
            {[...selectedRecipes].slice(0, 3).map((id, index) => <span key={id} style={{ transform: `translateX(${index * 7}px)` }}><Package size={16} /></span>)}
          </div>
          <div><strong>{t("{count} selected", { count: formatNumber(selectedCount) })}</strong><span>{t("Review dependencies and safeguards before applying.")}</span></div>
          <div className="selection-actions">
            <Button variant="ghost" onClick={() => onSetRecipeSelection([...selectedRecipes], false)} aria-label={t("Deselect all selected components")}>{t("Deselect all")}</Button>
            <Button variant="primary" onClick={onReview}>{t("Review changes")} <ArrowRight className="directional-icon" weight="bold" /></Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

interface CatalogRowProps {
  item: RecipeListItem;
  active: boolean;
  queued: boolean;
  compatibilityHostReady: boolean;
  hasPrefix: boolean;
  onSelect: (id: string) => void;
  onToggle: (id: string) => void;
}

const CatalogRow = memo(function CatalogRow({ item, active, queued, compatibilityHostReady, hasPrefix, onSelect, onToggle }: CatalogRowProps) {
  const { t } = useI18n();
  const executable = item.maturity === "native" || (item.maturity === "metadata_only" && compatibilityHostReady);
  const installable = hasPrefix && executable && item.compatible;
  return (
    <div className={clsx("catalog-row", active && "active")}>
      <button className="catalog-row-main" onClick={() => onSelect(item.id)}>
        <span className="category-icon"><CategoryIcon category={item.category} /></span>
        <span className="catalog-row-copy">
          <span className="catalog-row-title"><strong dir="auto">{item.title}</strong>{item.installed ? <CheckCircle size={15} weight="fill" /> : null}</span>
          <small dir="auto">{item.id}{item.publisher ? ` · ${item.publisher}` : ""}</small>
          <span className="row-badges">
            {item.cached ? <Badge tone="accent">{t("Cached")}</Badge> : null}
            {item.maturity === "metadata_only" && compatibilityHostReady ? <Badge tone="accent">{t("Winetricks host")}</Badge> : null}
            {!item.compatible ? <Badge tone="warning">{t("Incompatible")}</Badge> : null}
          </span>
        </span>
      </button>
      <button
        className={clsx("queue-toggle", queued && "queued")}
        onClick={() => onToggle(item.id)}
        disabled={!installable}
        aria-label={queued ? t("Remove {component} from selection", { component: item.title }) : t("Add {component} to selection", { component: item.title })}
        title={!hasPrefix ? t("Select a Wine prefix first") : item.maturity === "broken_upstream" ? t("This recipe is broken upstream") : item.maturity === "metadata_only" && !compatibilityHostReady ? t("Install the Winetricks release matching this catalog") : !item.compatible ? item.compatibility_reason ?? t("Not compatible with this prefix") : undefined}
      >
        {queued ? <Check size={15} weight="bold" /> : <Plus size={15} weight="bold" />}
      </button>
    </div>
  );
});

function RecipeDetail({ recipe, item, queued, prefix, compatibilityHostReady, compatibilityHostInstalling, onInstallCompatibilityHost, onToggle }: { recipe: Recipe; item?: RecipeListItem; queued: boolean; prefix: WinePrefix | null; compatibilityHostReady: boolean; compatibilityHostInstalling: boolean; onInstallCompatibilityHost: () => void; onToggle: () => void }) {
  const { t, formatNumber } = useI18n();
  const hosted = recipe.maturity === "metadata_only" && compatibilityHostReady;
  const executable = recipe.maturity === "native" || hosted;
  const installable = Boolean(prefix) && executable && item?.compatible !== false;
  return (
    <div className="recipe-detail">
      <div className="recipe-detail-top">
        <span className="detail-category-icon"><CategoryIcon category={recipe.category} size={22} /></span>
        <div className="detail-badges">
          <MaturityBadge maturity={recipe.maturity} />
          {item?.installed ? <Badge tone="success">{t("Installed")}</Badge> : null}
          {item?.cached ? <Badge tone="accent">{t("Ready offline")}</Badge> : null}
        </div>
      </div>
      <h2 dir="auto">{recipe.title}</h2>
      <div className="recipe-identity">
        <code>{recipe.id}</code>
        {recipe.publisher ? <span>{recipe.publisher}</span> : null}
        {recipe.year ? <span>{recipe.year}</span> : null}
      </div>
      <p className="recipe-description" dir="auto">{recipe.description ?? t("No description is available for this recipe.")}</p>

      {!installable ? (
        <div className="port-status-notice">
          <WarningCircle size={18} weight="fill" />
          <div>
            <strong>{!prefix ? t("Select a Wine prefix") : item?.compatible === false ? t("Not compatible with this prefix") : recipe.maturity === "broken_upstream" ? t("Broken upstream") : t("Matching Winetricks host required")}</strong>
            <span dir="auto">{!prefix ? t("Choose or add a prefix before building an operation plan.") : item?.compatibility_reason ?? (recipe.maturity === "broken_upstream" ? recipe.constraints.broken_reason : t("Install the checksum-verified Winetricks {version} host to run this tracked recipe while its native port remains in progress.", { version: recipe.source.upstream_tag }))}</span>
            {recipe.maturity === "metadata_only" && !compatibilityHostReady ? (
              <Button size="small" onClick={onInstallCompatibilityHost} disabled={compatibilityHostInstalling}>
                <CloudArrowDown /> {compatibilityHostInstalling ? t("Installing…") : t("Install verified host")}
              </Button>
            ) : null}
          </div>
        </div>
      ) : hosted ? (
        <div className="compatibility-ok">
          <ShieldCheck size={18} weight="fill" />
          <span><strong>{t("Available through Winetricks {version}", { version: recipe.source.upstream_tag })}</strong><small>{t("Bettertricks keeps prefix locking, activity tracking, cancellation, and recovery around the upstream behavior.")}</small></span>
        </div>
      ) : (
        <div className="compatibility-ok">
          <ShieldCheck size={18} weight="fill" />
          <span><strong>{t("Compatible with {prefix}", { prefix: prefix?.name ?? t("selected prefix") })}</strong><small>{prefix?.runtime_label ?? t("Wine runtime will be detected at launch")}</small></span>
        </div>
      )}

      <div className="detail-section">
        <h3>{t("What will happen")}</h3>
        {recipe.steps.length ? (
          <ol className="step-preview">
            {recipe.steps.map((step, index) => (
              <li key={`${step.type}-${index}`}><span>{formatNumber(index + 1)}</span><div><strong>{stepTitle(step, t)}</strong><small>{stepDescription(step, t)}</small></div></li>
            ))}
          </ol>
        ) : recipe.maturity === "metadata_only" ? (
          <ol className="step-preview"><li><span>{formatNumber(1)}</span><div><strong>{t("Run {recipe} through Winetricks", { recipe: recipe.id })}</strong><small>{t("The checksum-verified host must exactly match catalog baseline {version}.", { version: recipe.source.upstream_tag })}</small></div></li></ol>
        ) : (
          <p className="quiet-copy">{t("Execution steps become visible when the native recipe is complete.")}</p>
        )}
      </div>

      {(recipe.dependencies.length > 0 || recipe.conflicts.length > 0) ? (
        <div className="detail-section detail-grid">
          <div><h3>{t("Dependencies")}</h3><p>{recipe.dependencies.length ? recipe.dependencies.join(", ") : t("None")}</p></div>
          <div><h3>{t("Conflicts")}</h3><p>{recipe.conflicts.length ? recipe.conflicts.join(", ") : t("None known")}</p></div>
        </div>
      ) : null}

      {recipe.files.length > 0 ? (
        <div className="detail-section">
          <h3>{t("Downloads")}</h3>
          {recipe.files.map((file) => (
            <div className="download-file" key={file.id}>
              <CloudArrowDown size={18} />
              <span><strong>{file.filename}</strong><small>{file.manual ? t("Manual download") : t("Checksum verified automatically")}</small></span>
            </div>
          ))}
        </div>
      ) : null}

      <div className="recipe-source">
        <span>{t("Source")}</span>
        <strong>Winetricks {recipe.source.upstream_tag}</strong>
        <code>{recipe.source.upstream_verb}</code>
      </div>

      <div className="recipe-detail-action">
        <Button variant={queued ? "secondary" : "primary"} onClick={onToggle} disabled={!installable}>
          {queued ? <><X /> {t("Remove from plan")}</> : <><Check /> {t("Add to plan")}</>}
        </Button>
      </div>
    </div>
  );
}

function FilterButton({ active, children, onClick }: { active: boolean; children: React.ReactNode; onClick: () => void }) {
  return <button className={clsx("filter-button", active && "active")} aria-pressed={active} onClick={onClick}>{active ? <Check size={13} weight="bold" /> : null}{children}</button>;
}

function CatalogSkeleton() {
  return <div className="catalog-row skeleton-row"><Skeleton className="skeleton-icon" /><div><Skeleton className="skeleton-title" /><Skeleton className="skeleton-copy" /></div></div>;
}

function RecipeDetailSkeleton() {
  return <div className="recipe-detail"><Skeleton className="detail-icon-skeleton" /><Skeleton className="detail-title-skeleton" /><Skeleton className="detail-copy-skeleton" /><Skeleton className="detail-copy-skeleton short" /><Skeleton className="detail-block-skeleton" /></div>;
}

function stepTitle(step: Recipe["steps"][number], t: (key: string, values?: TranslationValues) => string) {
  if (step.type === "windows_version") return t("Set Windows version to {version}", { version: String(step.version) });
  if (step.type === "native_action" && step.action === "font_smoothing") return t("Update font rendering registry keys");
  return titleCase(step.type);
}

function stepDescription(step: Recipe["steps"][number], t: (key: string, values?: TranslationValues) => string) {
  if (step.type === "windows_version") return t("Wine applies the setting to the current prefix.");
  if (step.type === "download") return t("Download to the shared, checksum-verified cache.");
  return t("Applied by the native Bettertricks engine.");
}

function categoryLabel(category: VerbCategory | "all", t: (key: string, values?: TranslationValues) => string) {
  if (category === "all") return t("All");
  if (category === "dlls") return t("Components");
  if (category === "fonts") return t("Fonts");
  if (category === "apps") return t("Applications");
  if (category === "settings") return t("Settings");
  return t("Benchmarks");
}
