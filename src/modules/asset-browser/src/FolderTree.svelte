<script lang="ts">
  import { DocFolderTree, type FolderTreeLabels, type FolderDeleteChoice } from "@shadowcat/ui-kit";
  import { deleteAssetFolder, type DragTarget, type DropResult, type WireDocument } from "@shadowcat/core";
  import { ASSET_FOLDER_DOC_TYPE, ROOT_DROP_ID } from "./folderOps";

  let {
    selectedFolder,
    onSelectFolder,
    onDropFiles,
    mutable = true,
    assetDropHover = null,
  }: {
    /** The folder filtering the grid, or `null` = all assets. */
    selectedFolder: string | null;
    /** Called when a node (or the all-assets row) is chosen. */
    onSelectFolder: (id: string | null) => void;
    /** OS files dropped onto a folder node (upload targets). */
    onDropFiles?: (files: File[], folderId: string) => void;
    /** Whether mutation affordances (create/move/delete) render; pick mode
     * passes `false`. Defaults to mutable. */
    mutable?: boolean;
    /** The drop resolution of an in-progress asset-tile drag (owned by the grid), used to
     * highlight the folder row or all-assets row it would land on. */
    assetDropHover?: DropResult | null;
  } = $props();

  /** The shared tree's labels for asset folders. */
  const LABELS: FolderTreeLabels = {
    all: "assetBrowser.allAssets",
    newFolder: "assetBrowser.newFolder",
    dragFolder: "assetBrowser.dragFolder",
    rename: "assetBrowser.renameFolder",
    move: "assetBrowser.moveFolder",
    delete: "assetBrowser.deleteFolder",
    moveToRoot: "assetBrowser.moveToRoot",
    deletePrompt: "assetBrowser.deleteFolderPrompt",
    cancel: "assetBrowser.pickCancel",
    error: "assetBrowser.error",
  };

  /** The delete dialog's two asset policies: keep the folder's assets (reparent) or purge them. */
  const DELETE_CHOICES: FolderDeleteChoice[] = [
    { id: "reparent", labelKey: "assetBrowser.deleteReparent" },
    { id: "purge", labelKey: "assetBrowser.deletePurge", danger: true },
  ];

  /** The shared tree instance, read for its drop targets. */
  let tree = $state<ReturnType<typeof DocFolderTree> | null>(null);

  /** The drop targets an asset-tile drag lands on: every folder row plus the all-assets row.
   * @returns The targets, rects read fresh on each call.
   * @example
   * ```
   * // exported; invoked by AssetBrowser through bind:this
   * declare const folderTree: { dropTargets(): DragTarget[] };
   * folderTree.dropTargets();
   * ```
   */
  export function dropTargets(): DragTarget[] {
    return tree?.dropTargets() ?? [];
  }

  /** Runs the folder delete with the dialog's chosen asset policy.
   * @param folder - The folder to delete.
   * @param choiceId - `reparent` (default UX) or `purge`.
   * @example
   * ```
   * // private function; passed to the shared tree as onDelete
   * declare const folder: WireDocument;
   * void deleteFolder(folder, "reparent");
   * ```
   */
  async function deleteFolder(folder: WireDocument, choiceId: string): Promise<void> {
    await deleteAssetFolder(folder.id, choiceId === "purge" ? "delete" : "reparent");
  }
</script>

<DocFolderTree
  bind:this={tree}
  folderDocType={ASSET_FOLDER_DOC_TYPE}
  labels={LABELS}
  rootDropId={ROOT_DROP_ID}
  deleteChoices={DELETE_CHOICES}
  onDelete={deleteFolder}
  {selectedFolder}
  {onSelectFolder}
  {onDropFiles}
  {mutable}
  dropHover={assetDropHover}
/>
