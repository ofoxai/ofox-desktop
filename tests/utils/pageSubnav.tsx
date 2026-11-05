import { SubNav } from "@/components/shell/AppSidebar";
import { useShell, type ShellPage } from "@/components/shell/ShellContext";

/**
 * The sidebar's sub-page tabs for one page, as a test renders them next
 * to a page without the whole sidebar.
 */
export function PageSubnav({ page }: { page: ShellPage }) {
  const nav = useShell().subnavs[page];
  return nav ? <SubNav label={`${page} sub-pages`} nav={nav} /> : null;
}
