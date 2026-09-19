import { forwardRef, useSyncExternalStore } from "react";
import * as Lucide from "lucide-react";
import type { LucideIcon, LucideProps } from "lucide-react";
import { effectiveTheme, subscribeTheme } from "./theme/runtime";
import { productIcons } from "./theme/product-icons";
import { productIconDefinition } from "./theme/icon-theme";
import { IconGlyph } from "./theme/IconGlyph";

function product(name: keyof typeof productIcons, Fallback: LucideIcon) {
  const Component = forwardRef<
    SVGSVGElement,
    LucideProps & { iconId?: string }
  >(function ProductIcon({ iconId, size = 24, ...props }, ref) {
    const theme = useSyncExternalStore(
      subscribeTheme,
      effectiveTheme,
    ).productIcons;
    const selected =
      theme &&
      productIconDefinition(
        theme.data,
        iconId ? [iconId, ...productIcons[name]] : productIcons[name],
      );
    if (!theme || !selected)
      return <Fallback ref={ref} size={size} {...props} />;
    return (
      <svg
        ref={ref}
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill="currentColor"
        aria-hidden="true"
        {...props}
        className={`lucide product-icon ${props.className ?? ""}`}
        data-product-icon={selected.id}
      >
        <IconGlyph
          theme={theme}
          definition={selected.definition}
          size={size}
          product
        />
      </svg>
    );
  });
  return Component;
}

export const ArrowDown = product("ArrowDown", Lucide.ArrowDown);
export const ArrowLeft = product("ArrowLeft", Lucide.ArrowLeft);
export const ArrowRight = product("ArrowRight", Lucide.ArrowRight);
export const ArrowUp = product("ArrowUp", Lucide.ArrowUp);
export const Camera = product("Camera", Lucide.Camera);
export const CaseSensitive = product("CaseSensitive", Lucide.CaseSensitive);
export const Check = product("Check", Lucide.Check);
export const ChevronDown = product("ChevronDown", Lucide.ChevronDown);
export const ChevronLeft = product("ChevronLeft", Lucide.ChevronLeft);
export const ChevronRight = product("ChevronRight", Lucide.ChevronRight);
export const ChevronsDownUp = product("ChevronsDownUp", Lucide.ChevronsDownUp);
export const ChevronsUpDown = product("ChevronsUpDown", Lucide.ChevronsUpDown);
export const CircleAlert = product("CircleAlert", Lucide.CircleAlert);
export const Circle = product("Circle", Lucide.Circle);
export const Code = product("Code", Lucide.Code);
export const Columns2 = product("Columns2", Lucide.Columns2);
export const Command = product("Command", Lucide.Command);
export const Copy = product("Copy", Lucide.Copy);
export const Download = product("Download", Lucide.Download);
export const Ellipsis = product("Ellipsis", Lucide.Ellipsis);
export const ExternalLink = product("ExternalLink", Lucide.ExternalLink);
export const Eye = product("Eye", Lucide.Eye);
export const EyeOff = product("EyeOff", Lucide.EyeOff);
export const File = product("File", Lucide.File);
export const FileCode = product("FileCode", Lucide.FileCode);
export const FileDiff = product("FileDiff", Lucide.FileDiff);
export const FileJson = product("FileJson", Lucide.FileJson);
export const FilePlus2 = product("FilePlus2", Lucide.FilePlus2);
export const FileText = product("FileText", Lucide.FileText);
export const Folder = product("Folder", Lucide.Folder);
export const FolderOpen = product("FolderOpen", Lucide.FolderOpen);
export const GitBranch = product("GitBranch", Lucide.GitBranch);
export const GitCommitHorizontal = product(
  "GitCommitHorizontal",
  Lucide.GitCommitHorizontal,
);
export const Globe = product("Globe", Lucide.Globe);
export const History = product("History", Lucide.History);
export const Import = product("Import", Lucide.Import);
export const Info = product("Info", Lucide.Info);
export const Keyboard = product("Keyboard", Lucide.Keyboard);
export const Layers = product("Layers", Lucide.Layers);
export const LayoutGrid = product("LayoutGrid", Lucide.LayoutGrid);
export const ListX = product("ListX", Lucide.ListX);
export const Maximize2 = product("Maximize2", Lucide.Maximize2);
export const Menu = product("Menu", Lucide.Menu);
export const Minimize2 = product("Minimize2", Lucide.Minimize2);
export const Minus = product("Minus", Lucide.Minus);
export const Monitor = product("Monitor", Lucide.Monitor);
export const Moon = product("Moon", Lucide.Moon);
export const Palette = product("Palette", Lucide.Palette);
export const PanelLeft = product("PanelLeft", Lucide.PanelLeft);
export const PanelRight = product("PanelRight", Lucide.PanelRight);
export const Pencil = product("Pencil", Lucide.Pencil);
export const Pin = product("Pin", Lucide.Pin);
export const PinOff = product("PinOff", Lucide.PinOff);
export const Play = product("Play", Lucide.Play);
export const Plus = product("Plus", Lucide.Plus);
export const Power = product("Power", Lucide.Power);
export const Puzzle = product("Puzzle", Lucide.Puzzle);
export const Redo2 = product("Redo2", Lucide.Redo2);
export const RefreshCw = product("RefreshCw", Lucide.RefreshCw);
export const Regex = product("Regex", Lucide.Regex);
export const RotateCcw = product("RotateCcw", Lucide.RotateCcw);
export const RotateCw = product("RotateCw", Lucide.RotateCw);
export const Save = product("Save", Lucide.Save);
export const Search = product("Search", Lucide.Search);
export const Settings = product("Settings", Lucide.Settings);
export const ShieldCheck = product("ShieldCheck", Lucide.ShieldCheck);
export const Smartphone = product("Smartphone", Lucide.Smartphone);
export const Square = product("Square", Lucide.Square);
export const SquareArrowRight = product(
  "SquareArrowRight",
  Lucide.SquareArrowRight,
);
export const SquareDot = product("SquareDot", Lucide.SquareDot);
export const SquareMinus = product("SquareMinus", Lucide.SquareMinus);
export const SquarePlus = product("SquarePlus", Lucide.SquarePlus);
export const Sun = product("Sun", Lucide.Sun);
export const Terminal = product("Terminal", Lucide.Terminal);
export const Trash2 = product("Trash2", Lucide.Trash2);
export const Undo2 = product("Undo2", Lucide.Undo2);
export const WholeWord = product("WholeWord", Lucide.WholeWord);
export const WrapText = product("WrapText", Lucide.WrapText);
export const X = product("X", Lucide.X);

export const MessageSquare = Lucide.MessageSquare;
export const Paperclip = Lucide.Paperclip;
export const Send = Lucide.Send;
