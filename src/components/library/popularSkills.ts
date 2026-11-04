import snapshot from "./popularSkills.json";

export type PopularSkill = {
  source: string;
  skillId: string;
  name: string;
  installs: number;
  official: boolean;
  description?: string;
};

const avatars = import.meta.glob<string>(
  "../../assets/library-logos/owners/*.png",
  { eager: true, query: "?url", import: "default" },
);

export function ownerAvatar(source: string): string | undefined {
  const owner = source.split("/")[0].toLowerCase();
  return avatars[`../../assets/library-logos/owners/${owner}.png`];
}

export function formatInstalls(n: number): string {
  if (n < 1000) return String(n);
  const divisor = n < 1_000_000 ? 1000 : 1_000_000;
  return `${(n / divisor).toFixed(1).replace(/\.0$/, "")}${divisor === 1000 ? "K" : "M"}`;
}

export const POPULAR_SKILLS: PopularSkill[] = [...snapshot.skills].sort(
  (a, b) => b.installs - a.installs,
);
