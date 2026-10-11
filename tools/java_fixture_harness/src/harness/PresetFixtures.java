package harness;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import fr.soe.a3s.domain.Addon;
import fr.soe.a3s.dto.TreeDirectoryDTO;
import fr.soe.a3s.dto.TreeLeafDTO;
import fr.soe.a3s.dto.TreeNodeDTO;
import fr.soe.a3s.service.Arma3LauncherPresetExporter;

/**
 * Arma 3 Launcher preset export cases ({@code java_presets/}): the real
 * {@code Arma3LauncherPresetExporter.generate} and {@code write} on small modsets.
 */
public final class PresetFixtures {

	/** One case: modset tree and the {@code meta.cpp} text per addon folder (null = no file). */
	private static final class Case {
		final String name;
		final TreeDirectoryDTO modset;
		final Map<String, String> metaCpp = new LinkedHashMap<String, String>();

		Case(String name, String presetName) {
			this.name = name;
			this.modset = new TreeDirectoryDTO();
			this.modset.setName(presetName);
		}

		TreeLeafDTO leaf(TreeDirectoryDTO parent, String leafName, boolean selected) {
			TreeLeafDTO leaf = new TreeLeafDTO();
			leaf.setName(leafName);
			leaf.setSelected(selected);
			leaf.setParent(parent);
			parent.addTreeNode(leaf);
			return leaf;
		}

		TreeDirectoryDTO group(TreeDirectoryDTO parent, String groupName) {
			TreeDirectoryDTO group = new TreeDirectoryDTO();
			group.setName(groupName);
			group.setSelected(true);
			group.setParent(parent);
			parent.addTreeNode(group);
			return group;
		}

		Case addon(String folder, String meta) {
			metaCpp.put(folder, meta);
			return this;
		}
	}

	private PresetFixtures() {
	}

	private static List<Case> cases() {
		List<Case> cases = new ArrayList<Case>();

		Case p1 = new Case("p1_golden", "Event <HS13>");
		p1.leaf(p1.modset, "@gm", true);
		p1.leaf(p1.modset, "WS", true);
		p1.addon("@gm", "publishedid = 3147580028;\nname = \"GM Enhancement & Intern\";\n");
		cases.add(p1);

		Case p2 = new Case("p2_no_cdlc_escaped", "Mods only");
		p2.leaf(p2.modset, "@quoted", true);
		p2.leaf(p2.modset, "@plain", true);
		p2.addon("@quoted", "protocol = 1;\npublishedid = 1111111111;\nname = \"Mod \\\"Quoted\\\" & Co\";\n");
		p2.addon("@plain", "publishedid = 2222222222;\nname = \"Plain Mod\";\ntimestamp = 5250609374435198798;\n");
		cases.add(p2);

		Case p3 = new Case("p3_duplicate_nested", "Nested");
		TreeDirectoryDTO group = p3.group(p3.modset, "Group");
		p3.leaf(group, "@inner", true);
		p3.leaf(p3.modset, "@outer", true);
		p3.leaf(p3.modset, "@same_id", true);
		p3.leaf(p3.modset, "@unselected", false);
		p3.leaf(p3.modset, "GM", true);
		p3.addon("@inner", "publishedid = 3333333333;\nname = \"Inner Mod\";\n");
		p3.addon("@outer", "publishedid = 4444444444;\nname = \"Outer Mod\";\n");
		p3.addon("@same_id", "publishedid = 4444444444;\nname = \"Same Id Mod\";\n");
		p3.addon("@unselected", "publishedid = 5555555555;\nname = \"Unselected Mod\";\n");
		cases.add(p3);

		Case p4 = new Case("p4_raw_lf_name", "Raw LF");
		p4.leaf(p4.modset, "@lf", true);
		p4.addon("@lf", "publishedid = 6666666666;\nname = \"First line\nSecond line\";\n");
		cases.add(p4);

		Case p5 = new Case("p5_missing_meta", "Missing meta");
		p5.leaf(p5.modset, "@no_meta", true);
		p5.addon("@no_meta", null);
		cases.add(p5);

		Case p6 = new Case("p6_name_fallback", "Fallback");
		p6.leaf(p6.modset, "@nameless", true);
		p6.addon("@nameless", "publishedid = 7777777777;\n");
		cases.add(p6);

		Case p7 = new Case("p7_zero_id", "Zero id");
		p7.leaf(p7.modset, "@zero", true);
		p7.addon("@zero", "publishedid = 0;\nname = \"Zero Id Mod\";\n");
		cases.add(p7);
		return cases;
	}

	static void generate(Path out, Path work) throws Exception {
		Path root = out.resolve("java_presets");
		for (Case c : cases()) {
			Path addons = work.resolve(c.name);
			Map<String, Addon> resolver = new LinkedHashMap<String, Addon>();
			List<Object> metaJson = new ArrayList<Object>();
			for (Map.Entry<String, String> entry : c.metaCpp.entrySet()) {
				Path folder = addons.resolve(entry.getKey());
				Files.createDirectories(folder);
				if (entry.getValue() != null) {
					Files.write(folder.resolve("meta.cpp"), entry.getValue().getBytes(StandardCharsets.UTF_8));
				}
				resolver.put(entry.getKey(), new Addon(entry.getKey(), entry.getKey(), addons.toString()));
				metaJson.add(Json.obj().put("folder", entry.getKey()).put("meta_cpp", entry.getValue()));
			}
			Path caseDir = root.resolve(c.name);
			Files.createDirectories(caseDir);
			Arma3LauncherPresetExporter exporter = new Arma3LauncherPresetExporter();
			// Mirrors service/Arma3LauncherPresetExporter.java:43-45, 77-105
			Arma3LauncherPresetExporter.ExportResult result = exporter.generate(c.modset, resolver::get);
			if (result.isValid()) {
				exporter.write(caseDir.resolve("preset.html"), result);
			}
			StringBuilder text = new StringBuilder();
			text.append("valid=").append(result.isValid()).append('\n');
			text.append("workshop=").append(result.getWorkshopCount()).append('\n');
			text.append("dlc=").append(result.getDlcCount()).append('\n');
			for (String error : result.getErrors()) {
				text.append("error: ").append(error).append('\n');
			}
			for (String warning : result.getWarnings()) {
				text.append("warning: ").append(warning).append('\n');
			}
			for (Arma3LauncherPresetExporter.MetadataIssue issue : result.getMetadataIssues()) {
				text.append("issue: ").append(issue.getAddonKey()).append('|').append(issue.getAddonName()).append('|')
						.append(issue.getReason()).append('|').append(issue.getPublishedId()).append('|')
						.append(issue.getName()).append('\n');
			}
			Io.writeString(caseDir.resolve("result.txt"), text.toString());
			Io.writeString(caseDir.resolve("case.json"), Json.document(Json.obj().put("case", c.name)
					.put("modset", tree(c.modset)).put("addons", metaJson)));
		}
	}

	private static Json.Obj tree(TreeNodeDTO node) {
		if (node.isLeaf()) {
			return Json.obj().put("leaf", node.getName()).put("selected", node.isSelected());
		}
		List<Object> children = new ArrayList<Object>();
		for (TreeNodeDTO child : ((TreeDirectoryDTO) node).getList()) {
			children.add(tree(child));
		}
		return Json.obj().put("directory", node.getName()).put("selected", node.isSelected()).put("children",
				children);
	}
}
