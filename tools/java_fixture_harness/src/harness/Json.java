package harness;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Minimal compact JSON writer. Values are {@code null}, {@link String}, {@link Boolean},
 * {@link Number}, {@link Json.Obj}, {@link Json.Raw} or a {@link List} of values. Strings use JSON
 * escaping with {@code \\uXXXX} for control characters; there is no whitespace.
 */
public final class Json {

	private Json() {
	}

	/** An object with keys in insertion order. */
	public static final class Obj {
		private final Map<String, Object> entries = new LinkedHashMap<String, Object>();

		/** Adds (or replaces) a key and returns {@code this}. */
		public Obj put(String key, Object value) {
			entries.put(key, value);
			return this;
		}

		@Override
		public String toString() {
			StringBuilder sb = new StringBuilder();
			write(sb, this);
			return sb.toString();
		}
	}

	/** Pre-rendered JSON text, inserted verbatim. */
	public static final class Raw {
		private final String text;

		public Raw(String text) {
			this.text = text;
		}
	}

	public static Obj obj() {
		return new Obj();
	}

	public static List<Object> list(Object... values) {
		List<Object> list = new ArrayList<Object>();
		for (Object value : values) {
			list.add(value);
		}
		return list;
	}

	/** Renders a value followed by a newline, the format of every JSON file the harness writes. */
	public static String document(Object value) {
		StringBuilder sb = new StringBuilder();
		write(sb, value);
		sb.append('\n');
		return sb.toString();
	}

	public static void write(StringBuilder sb, Object value) {
		if (value == null) {
			sb.append("null");
		} else if (value instanceof String) {
			string(sb, (String) value);
		} else if (value instanceof Boolean) {
			sb.append(((Boolean) value).booleanValue() ? "true" : "false");
		} else if (value instanceof Double) {
			sb.append(Double.toString((Double) value));
		} else if (value instanceof Float) {
			sb.append(Float.toString((Float) value));
		} else if (value instanceof Number) {
			sb.append(value.toString());
		} else if (value instanceof Raw) {
			sb.append(((Raw) value).text);
		} else if (value instanceof Obj) {
			sb.append('{');
			boolean first = true;
			for (Map.Entry<String, Object> entry : ((Obj) value).entries.entrySet()) {
				if (!first) {
					sb.append(',');
				}
				first = false;
				string(sb, entry.getKey());
				sb.append(':');
				write(sb, entry.getValue());
			}
			sb.append('}');
		} else if (value instanceof List) {
			sb.append('[');
			boolean first = true;
			for (Object element : (List<?>) value) {
				if (!first) {
					sb.append(',');
				}
				first = false;
				write(sb, element);
			}
			sb.append(']');
		} else {
			throw new IllegalArgumentException("not a JSON value: " + value.getClass().getName());
		}
	}

	public static void string(StringBuilder sb, String s) {
		sb.append('"');
		for (int i = 0; i < s.length(); i++) {
			char c = s.charAt(i);
			if (c == '"') {
				sb.append("\\\"");
			} else if (c == '\\') {
				sb.append("\\\\");
			} else if (c < 0x20) {
				sb.append(String.format("\\u%04x", (int) c));
			} else {
				sb.append(c);
			}
		}
		sb.append('"');
	}
}
