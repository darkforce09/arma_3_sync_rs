package harness;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

import static harness.LocalData.typed;

import fr.soe.a3s.dao.A3SFilesAccessor;
import fr.soe.a3s.dto.EventDTO;
import fr.soe.a3s.service.RepositoryService;

/**
 * Events session ({@code java_built/events_session/}): the GUI event calls of
 * {@code RepositoryService} on a freshly loaded repository, with {@code writeEvents} and a copy
 * of {@code .a3s/events} after every call.
 */
public final class EventsSession {

	static final String VARIANT = "http_plain";

	private EventsSession() {
	}

	static void generate(Path out, Path work) throws Exception {
		Main.runChild(null, Main.perUserProperties(work), "child-events", work.toString(), out.toString());
		for (String rel : Io.files(out)) {
			if (rel.endsWith("/events")) {
				String step = rel.substring(0, rel.indexOf('_'));
				Object events = A3SFilesAccessor.read(out.resolve(rel).toFile());
				Io.writeString(out.resolve("dumps").resolve(step + ".json"), JsonDump.dump(events, false));
			}
		}
	}

	/** {@code child-events <work> <out>}. */
	static void child(List<String> args) throws Exception {
		if (args.size() != 2) {
			throw new UsageException("usage: child-events <work> <out>");
		}
		Path work = Paths.get(args.get(0)).toAbsolutePath().normalize();
		Path out = Paths.get(args.get(1)).toAbsolutePath().normalize();
		Path repo = work.resolve(BuiltFixtures.FOLDER);
		String name = BuiltFixtures.repositoryName(VARIANT);

		RepositoryService service = new RepositoryService();
		BuiltFixtures.createSampleTree(repo);
		BuiltFixtures.setUpRepository(service, VARIANT, repo);
		BuiltFixtures.build(name, repo, BuiltFixtures.BUILD_DATES[0]);
		// Reload as after a restart: Repository.events is transient, so the session starts with null events.
		service.readAll();
		if (LocalData.repository(name).getEvents() != null) {
			throw new HarnessException("events are not null after readAll");
		}

		List<Object> steps = new ArrayList<Object>();

		EventDTO night = event("Night Op", "Night operation", new String[] { "@Mod_A", "@mod_b", "@Mod_D" },
				new boolean[] { true, false, true }, new String[] { "WS", "GM" }, new boolean[] { true, false },
				new String[0], new boolean[0]);
		service.addEvent(name, night);
		record(steps, service, name, repo, out, "s01_add", "addEvent", dto(night));

		EventDTO full = event("Night Op", "Night operation", new String[] { "@Mod_A", "@mod_b", "@Mod_D" },
				new boolean[] { true, true, false }, new String[] { "WS", "GM" }, new boolean[] { true, true },
				new String[] { "@Mod_A", "@mod_b" }, new boolean[] { true, false });
		service.saveEvent(name, full);
		record(steps, service, name, repo, out, "s02_save_full", "saveEvent", dto(full));

		EventDTO shrunk = event("Night Op", "Night operation", new String[] { "@Mod_A" }, new boolean[] { true },
				new String[0], new boolean[0], new String[] { "@Mod_A" }, new boolean[] { true });
		service.saveEvent(name, shrunk);
		record(steps, service, name, repo, out, "s03_save_shrunk", "saveEvent", dto(shrunk));

		service.duplicateEvent(name, "Night Op");
		record(steps, service, name, repo, out, "s04_duplicate", "duplicateEvent",
				Json.obj().put("event_name", "Night Op"));

		service.renameEvent(name, "Night Op - duplicate", typed("Day Op"), typed("desc"));
		record(steps, service, name, repo, out, "s05_rename", "renameEvent", Json.obj()
				.put("event_name", "Night Op - duplicate").put("new_event_name", "Day Op").put("new_description", "desc"));

		String[] addons = new String[13];
		boolean[] values = new boolean[13];
		for (int i = 0; i < addons.length; i++) {
			addons[i] = String.format("@Training%02d", i + 1);
			values[i] = i % 3 != 0;
		}
		EventDTO training = event("Training", "Weekly training", addons, values, new String[0], new boolean[0],
				new String[0], new boolean[0]);
		service.addEvent(name, training);
		record(steps, service, name, repo, out, "s06_add_second", "addEvent", dto(training));

		service.removeEvent(name, "Night Op");
		record(steps, service, name, repo, out, "s07_remove", "removeEvent", Json.obj().put("event_name", "Night Op"));

		Io.writeString(out.resolve("session.json"), Json.document(Json.obj().put("repository_name", name)
				.put("variant", VARIANT).put("build_date_ms", BuiltFixtures.BUILD_DATES[0]).put("steps", steps)));
	}

	private static EventDTO event(String name, String description, String[] addons, boolean[] addonValues,
			String[] dlcs, boolean[] dlcValues, String[] userconfig, boolean[] userconfigValues) {
		EventDTO dto = new EventDTO();
		dto.setName(typed(name));
		dto.setDescription(typed(description));
		for (int i = 0; i < addons.length; i++) {
			dto.getAddonNames().put(typed(addons[i]), addonValues[i]);
		}
		for (int i = 0; i < dlcs.length; i++) {
			dto.getDlcNames().put(typed(dlcs[i]), dlcValues[i]);
		}
		for (int i = 0; i < userconfig.length; i++) {
			dto.getUserconfigFolderNames().put(typed(userconfig[i]), userconfigValues[i]);
		}
		return dto;
	}

	/** The DTO exactly as the service sees it (map iteration order). */
	private static Json.Obj dto(EventDTO dto) {
		return Json.obj().put("name", dto.getName()).put("description", dto.getDescription())
				.put("addon_names", entries(dto.getAddonNames())).put("dlc_names", entries(dto.getDlcNames()))
				.put("userconfig_folder_names", entries(dto.getUserconfigFolderNames()));
	}

	private static List<Object> entries(Map<String, Boolean> map) {
		List<Object> entries = new ArrayList<Object>();
		for (Map.Entry<String, Boolean> entry : map.entrySet()) {
			entries.add(Json.list(entry.getKey(), entry.getValue()));
		}
		return entries;
	}

	private static void record(List<Object> steps, RepositoryService service, String name, Path repo, Path out,
			String step, String call, Json.Obj arguments) throws Exception {
		// Mirrors service/RepositoryService.java:1061-1067
		service.writeEvents(name);
		Path target = out.resolve(step).resolve("events");
		Files.createDirectories(target.getParent());
		Files.copy(repo.resolve(".a3s").resolve("events"), target, StandardCopyOption.REPLACE_EXISTING);
		steps.add(Json.obj().put("name", step).put("call", call).put("arguments", arguments)
				.put("events", step + "/events"));
	}
}
