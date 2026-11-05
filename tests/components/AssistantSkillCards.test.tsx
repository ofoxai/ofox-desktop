import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { SkillResultCards } from "@/components/assistant/SkillResultCards";
import { LocalApprovalCard } from "@/components/assistant/AssistantLocalConfig";

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
it("install approval shows the frozen source and only installs on click", async () => {
  const onRespond = vi.fn().mockResolvedValue(true);
  const approval = JSON.parse(
    JSON.stringify({
      approvalId: "a",
      toolCallId: "c",
      name: "install_skill",
      category: "skillInstall",
      details: {
        name: "fixture-skill",
        source: "fixture/repo",
        directory: "nested/fixture",
        commitSha: "a".repeat(40),
        description: "Fixture",
        reviewed: false,
        installs: 42,
      },
      allowSession: false,
      expectedOwner: "o",
      expectedGeneration: 1,
    }),
  );
  render(<LocalApprovalCard approval={approval} onRespond={onRespond} />);
  expect(screen.getByText("fixture/repo")).toBeInTheDocument();
  expect(screen.getByText("nested/fixture")).toBeInTheDocument();
  expect(onRespond).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: /session/i })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Install" }));
  expect(onRespond).toHaveBeenCalledWith("a", "once");
});
it("save approval previews Markdown without loading images or accepting raw HTML", async () => {
  const onRespond = vi.fn().mockResolvedValue(true);
  const approval = JSON.parse(
    JSON.stringify({
      approvalId: "a",
      toolCallId: "c",
      name: "save_skill",
      category: "skillSave",
      details: {
        name: "fixture-skill",
        description: "Fixture description",
        body: "# Fixture instructions\n\n![remote](https://invalid.test/image)\n<script>bad()</script>",
      },
      allowSession: false,
      expectedOwner: "o",
      expectedGeneration: 1,
    }),
  );
  const { container } = render(
    <LocalApprovalCard approval={approval} onRespond={onRespond} />,
  );
  expect(
    screen.getByRole("heading", { name: "Fixture instructions" }),
  ).toBeInTheDocument();
  expect(container.querySelector("img,script,iframe")).toBeNull();
  expect(onRespond).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(onRespond).toHaveBeenCalledWith("a", "once");
});

it("historic search cards are bounded and have no mutation controls", async () => {
  const result = {
    resultId: "old",
    name: "fixture-skill",
    source: "fixture/repo",
    description: "Description",
    installs: 42,
    reviewed: false,
  };
  const { container } = render(
    <SkillResultCards
      groups={[{ toolCallId: "old-search", results: [result] }]}
    />,
  );
  expect(screen.getByText("fixture-skill")).toBeInTheDocument();
  expect(container.querySelector("button,a,input,select,form")).toBeNull();
  await userEvent.click(screen.getByText("fixture-skill"));
  expect(container.querySelector("button")).toBeNull();
});

it("a committed install is displayed as Installed in its read-only search card", () => {
  const groups = JSON.parse(
    JSON.stringify([
      {
        toolCallId: "search",
        results: [
          {
            resultId: "r",
            name: "fixture",
            source: "fixture/repo",
            description: "Fixture",
            reviewed: false,
            installed: true,
          },
        ],
      },
    ]),
  );
  const { container } = render(<SkillResultCards groups={groups} />);
  expect(screen.getByText("Installed", { exact: true })).toBeInTheDocument();
  expect(container.querySelector("button,a")).toBeNull();
});
