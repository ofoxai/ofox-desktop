import path from "node:path";
import { validatePlan } from "./local-task.mjs";
import { Type } from "typebox";
import {
  createReadToolDefinition,
  createLsToolDefinition,
  createFindToolDefinition,
  createWriteToolDefinition,
  createEditToolDefinition,
  createBashToolDefinition,
  createPowerShellToolDefinition,
} from "@earendil-works/pi-coding-agent";

const textResult = (value) => ({
  content: [
    {
      type: "text",
      text: typeof value === "string" ? value : JSON.stringify(value),
    },
  ],
  details: value,
});

// Pi keeps its formatting and line/glob selection. All operations below cross
// the host bridge; none uses Node filesystem/process APIs or Pi's default ops.
export function localTools(runtime, context) {
  const cwd = context.params.localConfig.cwd;
  const make = (factory) => {
    const schema = factory({}).parameters;
    const sample = factory({});
    const parameters = Type.Object({
      ...schema.properties,
      intent: Type.Optional(Type.String({ maxLength: 200 })),
    });
    const tool = runtime.wrapTool(
      context,
      sample.name,
      sample.description,
      parameters,
      async (args, signal, id) => {
        let sequence = 0;
        const cache = new Map();
        const host = (name, args) => {
          const key = JSON.stringify([name, args]);
          if (!cache.has(key))
            cache.set(
              key,
              runtime.hostTool(
                context,
                name,
                args,
                signal,
                `${id}:${++sequence}`,
              ),
            );
          return cache.get(key);
        };
        const stats = new Map();
        const stat = async (file) => {
          if (!stats.has(file))
            stats.set(file, await host("stat", { path: file }));
          return stats.get(file);
        };
        const observed = new Map();
        const operations = {
          access: async (file) => {
            if (!(await stat(file)).exists)
              throw new Error("assistant_file_unreadable");
          },
          readFile: async (file) => {
            const text = await host("read", { path: file });
            observed.set(file, text);
            return Buffer.from(text, "utf8");
          },
          // Parent creation belongs to the host's journaled write transaction.
          mkdir: async () => {},
          writeFile: async (file, content) => {
            await host(sample.name, {
              path: file,
              content,
              expected: observed.get(file) ?? null,
            });
          },
          detectImageMimeType: async () => null,
          exists: async (file) => (await stat(file)).exists,
          stat: async (file) => {
            const value = await stat(file);
            return { isDirectory: () => Boolean(value.directory) };
          },
          readdir: async (file) => {
            const entries = await host("ls", { path: file, limit: 1000 });
            for (const entry of entries)
              stats.set(path.join(file, entry.name), {
                exists: true,
                directory: entry.directory,
              });
            return entries.map((entry) => entry.name);
          },
          glob: (pattern, root, options) =>
            host("find", {
              path: root,
              pattern,
              limit: Math.min(options.limit, 1000),
            }),
        };
        const { intent, ...input } = args;
        return factory(operations).execute(id, input, signal);
      },
    );
    return { ...tool, promptGuidelines: sample.promptGuidelines };
  };
  const tools = [
    make((operations) =>
      createReadToolDefinition(cwd, { operations, autoResizeImages: false }),
    ),
    make((operations) => createLsToolDefinition(cwd, { operations })),
    make((operations) => createFindToolDefinition(cwd, { operations })),
    make((operations) => createWriteToolDefinition(cwd, { operations })),
    make((operations) => createEditToolDefinition(cwd, { operations })),
  ];
  const shellFactory =
    context.params.localConfig.platform === "windows"
      ? createPowerShellToolDefinition
      : createBashToolDefinition;
  const shellSample = shellFactory(cwd, {
    operations: {},
    exposeSessionEnvironment: false,
  });
  tools.push({
    ...runtime.wrapTool(
      context,
      shellSample.name,
      shellSample.description,
      Type.Object({
        ...shellSample.parameters.properties,
        folder: Type.Optional(Type.String()),
        intent: Type.Optional(Type.String({ maxLength: 200 })),
      }),
      async (args, signal, id) => {
        const folder = path.resolve(cwd, args.folder ?? ".");
        const operations = {
          exec: async (command, requested, options) => {
            const result = await runtime.hostTool(
              context,
              shellSample.name,
              {
                command,
                folder: requested,
                timeout: Math.min(
                  600,
                  Math.max(1, Math.floor(options.timeout ?? 60)),
                ),
              },
              options.signal,
              id,
            );
            // Rust caps output below Pi's spill thresholds (32 KiB / 800 lines),
            // so Pi never creates a plaintext output temp file in the runtime.
            options.onData(Buffer.from(result.output, "utf8"));
            return { exitCode: result.exitCode };
          },
        };
        return shellFactory(folder, {
          operations,
          exposeSessionEnvironment: false,
        }).execute(
          id,
          { command: args.command, timeout: args.timeout },
          signal,
        );
      },
    ),
    promptGuidelines: [
      `To list, find, search or read files use ls, find, grep and read: they need no approval. A ${shellSample.name} command may wait up to two minutes for the user to approve it; after assistant_approval_timeout nobody is there to approve, so do not ask again and go on another way.`,
    ],
  });
  const direct = (name, description, properties, transform = (args) => args) =>
    runtime.wrapTool(
      context,
      name,
      description,
      Type.Object({
        ...properties,
        intent: Type.Optional(Type.String({ maxLength: 200 })),
      }),
      async (args, signal, id) => {
        const { intent, ...input } = args;
        return textResult(
          await runtime.hostTool(context, name, transform(input), signal, id),
        );
      },
    );
  tools.push(
    direct(
      "grep",
      "Search redacted text through the host; never execute rg in the runtime.",
      {
        pattern: Type.String({ maxLength: 512 }),
        path: Type.Optional(Type.String()),
        limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 500 })),
      },
      (args) => ({
        ...args,
        path: path.resolve(cwd, args.path ?? "."),
        limit: args.limit ?? 100,
      }),
    ),
  );
  tools.push(
    direct(
      "tool_status",
      "Inspect a supported AI tool's installation and binding without modifying it.",
      { tool: Type.String() },
    ),
  );
  for (const name of ["diagnose_in_folder", "verify_in_folder"])
    tools.push(
      direct(
        name,
        "Check the tool as it starts in a folder: the user's home folder unless the user named the folder where it fails. After a repair, verification must succeed there.",
        { tool: Type.String(), folder: Type.Optional(Type.String()) },
        (args) => ({ ...args, folder: path.resolve(cwd, args.folder ?? ".") }),
      ),
    );
  for (const name of [
    "repair_launcher",
    "rebind_tool",
    "install_tool",
    "update_tool",
    "uninstall_tool",
  ])
    tools.push(
      direct(
        name,
        "Perform a host-managed tool operation. Installation, updates and removal always need individual approval and cannot be undone.",
        { tool: Type.String() },
      ),
    );
  tools.push(
    direct(
      "set_tool_model",
      "Set a supported model from the current region's catalog, preserving credentials and unrelated settings.",
      { tool: Type.String(), model: Type.String() },
    ),
  );
  tools.push(
    runtime.wrapTool(
      context,
      "updatePlan",
      "Keep a concise task plan, at most 12 steps and one in progress.",
      Type.Object({
        steps: Type.Array(
          Type.Object({
            step: Type.String({ maxLength: 40 }),
            status: Type.Union([
              Type.Literal("pending"),
              Type.Literal("in_progress"),
              Type.Literal("completed"),
            ]),
          }),
          { maxItems: 12 },
        ),
        intent: Type.Optional(Type.String({ maxLength: 200 })),
      }),
      async (args) => {
        const steps = validatePlan(args.steps);
        runtime.notify(context, "task_plan", { steps });
        return textResult({ steps });
      },
    ),
  );
  return tools;
}
