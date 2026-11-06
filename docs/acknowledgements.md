# Sources, licenses, and acknowledgements

Ofox Desktop is distributed under the [MIT license](../LICENSE). The existing
license text, copyright notices, and notices beside adapted code remain part of
the project.

## Upstream code

| Project                                              | Relationship                                                                                                                                                                                       | Preserved license and copyright                                                                                                                                                                                                                              |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [cc-switch](https://github.com/farion1231/cc-switch) | Ofox Desktop was developed from cc-switch and extends its tool/provider configuration foundation with Ofox accounts, tool management, resources, sessions, usage, and the assistant.               | MIT, Copyright © 2025 Jason Young. See the [repository license](../LICENSE) and existing source notices.                                                                                                                                                    |
| [magpie](https://github.com/yetone/magpie)           | Portions of Windows CLI discovery, installation, and updating, plus native tool and session integration, are adapted from Magpie. Individual module comments identify the adapted implementations. | MIT, Copyright © 2026 yetone. Full notices are retained with [native tools](../src-tauri/src/native_tools/MAGPIE_LICENSE), [session management](../src-tauri/src/session_manager/MAGPIE_LICENSE), and [services](../src-tauri/src/services/MAGPIE_LICENSE). |

These relationships include reused or adapted code. Thank you to the upstream
authors and contributors for making that work available.

## Tool and resource marks

Product names and marks identify their respective tools and publishers. Rights
remain with their owners. The selected files, origins, and existing notices are
recorded in:

- [Product mark sources](brand-assets.md).
- [Agent icon notices](../src/assets/agent-logos/NOTICE.md), including the retained
  [Magpie MIT notice](../src/assets/agent-logos/LICENSE-Magpie.txt) and
  [Lobe Icons MIT notice](../src/assets/agent-logos/LICENSE-Lobe-Icons.txt).
- [MCP resource icon sources](../src/assets/library-logos/NOTICE.md).

The corresponding license texts and copyright notices remain in their source
directories. A mark's appearance does not imply endorsement.

## README presentation

The README presentation was informed by the structure of
[OfficeCLI](https://github.com/iOfficeAI/OfficeCLI) and
[OmniRoute](https://github.com/cheaper-inference/OmniRoute). The showcase layouts,
flow diagrams, and synthetic UI captures were made for this repository. They
reuse Ofox's existing product mark and tool/resource assets with the provenance
above.

[README media notes](../assets/readme/README.md) and the
[showcase guide](../scripts/readme-showcase.md) describe the fictional data,
source baseline, capture process, and verification boundaries.
