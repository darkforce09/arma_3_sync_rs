package harness;

import java.io.ObjectStreamClass;
import java.io.ObjectStreamField;
import java.io.Serializable;
import java.lang.reflect.Field;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Date;
import java.util.Deque;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Canonical JSON view of a deserialized Arma3Sync object graph (README "Canonical JSON dump").
 *
 * <p>Identity mode gives every non-null object (strings included) an {@code "id"} in depth-first
 * order of first visit, the order {@code ObjectOutputStream} uses, and writes repeat visits as
 * {@code {"ref":N}}. Semantic mode drops identities, writes back-pointers to the enclosing object
 * as {@code {"parent":true}} and sorts set elements and map entries by their JSON text.
 */
public final class JsonDump {

	private final boolean semantic;
	private final Map<Object, Integer> ids = new IdentityHashMap<Object, Integer>();
	private final Deque<Object> expanding = new ArrayDeque<Object>();

	private JsonDump(boolean semantic) {
		this.semantic = semantic;
	}

	/** Dumps {@code root} as one JSON document followed by a newline. */
	public static String dump(Object root, boolean semantic) throws HarnessException {
		JsonDump dump = new JsonDump(semantic);
		StringBuilder sb = new StringBuilder();
		dump.value(sb, root);
		sb.append('\n');
		return sb.toString();
	}

	private void value(StringBuilder sb, Object value) throws HarnessException {
		if (value == null) {
			sb.append("null");
			return;
		}
		if (!semantic) {
			Integer known = ids.get(value);
			if (known != null) {
				sb.append("{\"ref\":").append(known).append('}');
				return;
			}
		}
		if (value instanceof String) {
			if (semantic) {
				Json.string(sb, (String) value);
			} else {
				sb.append("{\"str\":");
				Json.string(sb, (String) value);
				sb.append(",\"id\":").append(assign(value)).append('}');
			}
		} else if (value instanceof Enum) {
			sb.append("{\"enum\":");
			Json.string(sb, ((Enum<?>) value).getDeclaringClass().getName());
			sb.append(",\"name\":");
			Json.string(sb, ((Enum<?>) value).name());
			id(sb, value);
			sb.append('}');
		} else if (value instanceof Boolean || value instanceof Number || value instanceof Character) {
			boxed(sb, value);
		} else if (value instanceof Date) {
			sb.append("{\"class\":\"java.util.Date\",\"time\":").append(((Date) value).getTime());
			id(sb, value);
			sb.append('}');
		} else if (value instanceof List) {
			collection(sb, value, (List<?>) value, false);
		} else if (value instanceof Set) {
			collection(sb, value, (Set<?>) value, true);
		} else if (value instanceof Map) {
			map(sb, (Map<?, ?>) value);
		} else if (value.getClass().getName().startsWith("fr.soe.a3s.") && value instanceof Serializable) {
			domainObject(sb, value);
		} else {
			throw new HarnessException("unsupported class " + value.getClass().getName());
		}
	}

	private int assign(Object value) {
		int id = ids.size() + 1;
		ids.put(value, id);
		return id;
	}

	private void id(StringBuilder sb, Object value) {
		if (!semantic) {
			sb.append(",\"id\":").append(assign(value));
		}
	}

	private void boxed(StringBuilder sb, Object value) {
		sb.append("{\"class\":");
		Json.string(sb, value.getClass().getName());
		sb.append(",\"value\":");
		if (value instanceof Character) {
			Json.string(sb, value.toString());
		} else {
			Json.write(sb, value);
		}
		id(sb, value);
		sb.append('}');
	}

	private void collection(StringBuilder sb, Object value, Iterable<?> elements, boolean sortable)
			throws HarnessException {
		sb.append("{\"class\":");
		Json.string(sb, value.getClass().getName());
		id(sb, value);
		sb.append(",\"elements\":[");
		List<String> rendered = new ArrayList<String>();
		for (Object element : elements) {
			StringBuilder item = new StringBuilder();
			value(item, element);
			rendered.add(item.toString());
		}
		if (semantic && sortable) {
			Collections.sort(rendered);
		}
		sb.append(String.join(",", rendered)).append("]}");
	}

	private void map(StringBuilder sb, Map<?, ?> map) throws HarnessException {
		sb.append("{\"class\":");
		Json.string(sb, map.getClass().getName());
		id(sb, map);
		sb.append(",\"entries\":[");
		List<String> rendered = new ArrayList<String>();
		for (Map.Entry<?, ?> entry : map.entrySet()) {
			StringBuilder item = new StringBuilder("[");
			value(item, entry.getKey());
			item.append(',');
			value(item, entry.getValue());
			item.append(']');
			rendered.add(item.toString());
		}
		if (semantic) {
			Collections.sort(rendered);
		}
		sb.append(String.join(",", rendered)).append("]}");
	}

	private void domainObject(StringBuilder sb, Object value) throws HarnessException {
		if (semantic && isExpanding(value)) {
			if (expanding.size() >= 2 && second(expanding) == value) {
				sb.append("{\"parent\":true}");
				return;
			}
			throw new HarnessException("unsupported cycle through " + value.getClass().getName());
		}
		sb.append("{\"class\":");
		Json.string(sb, value.getClass().getName());
		id(sb, value);
		sb.append(",\"fields\":{");
		expanding.push(value);
		try {
			boolean first = true;
			for (Field field : serialFields(value.getClass())) {
				if (!first) {
					sb.append(',');
				}
				first = false;
				Json.string(sb, field.getName());
				sb.append(':');
				Object fieldValue;
				try {
					fieldValue = field.get(value);
				} catch (IllegalAccessException e) {
					throw new HarnessException("cannot read " + field, e);
				}
				if (field.getType().isPrimitive()) {
					if (fieldValue instanceof Character) {
						Json.string(sb, fieldValue.toString());
					} else {
						Json.write(sb, fieldValue);
					}
				} else {
					value(sb, fieldValue);
				}
			}
		} finally {
			expanding.pop();
		}
		sb.append("}}");
	}

	private boolean isExpanding(Object value) {
		for (Object current : expanding) {
			if (current == value) {
				return true;
			}
		}
		return false;
	}

	private static Object second(Deque<Object> stack) {
		java.util.Iterator<Object> it = stack.iterator();
		it.next();
		return it.next();
	}

	/**
	 * The serializable fields of every serializable class of the hierarchy, superclass first, each
	 * class in {@link ObjectStreamClass#getFields()} order (wire order).
	 */
	static List<Field> serialFields(Class<?> type) throws HarnessException {
		List<Class<?>> chain = new ArrayList<Class<?>>();
		for (Class<?> c = type; c != null && Serializable.class.isAssignableFrom(c); c = c.getSuperclass()) {
			chain.add(0, c);
		}
		List<Field> fields = new ArrayList<Field>();
		List<String> names = new ArrayList<String>();
		for (Class<?> c : chain) {
			ObjectStreamClass osc = ObjectStreamClass.lookup(c);
			if (osc == null) {
				continue;
			}
			for (ObjectStreamField osf : osc.getFields()) {
				if (names.contains(osf.getName())) {
					throw new HarnessException("duplicate field " + osf.getName() + " in " + type.getName());
				}
				try {
					Field field = c.getDeclaredField(osf.getName());
					field.setAccessible(true);
					fields.add(field);
					names.add(osf.getName());
				} catch (NoSuchFieldException e) {
					throw new HarnessException("no field " + osf.getName() + " in " + c.getName(), e);
				}
			}
		}
		return fields;
	}
}
