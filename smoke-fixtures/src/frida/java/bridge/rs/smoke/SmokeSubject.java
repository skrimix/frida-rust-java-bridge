package frida.java.bridge.rs.smoke;

public final class SmokeSubject {
    public static final String STATIC_TEXT = "static-smoke";
    private static int voidCounter = 0;
    public int number = 7;
    private long hidden = 11L;

    public SmokeSubject() {
    }

    public SmokeSubject(int number) {
        this.number = number;
    }

    public String message() {
        return "dex-smoke";
    }

    public int instanceNumber() {
        return number;
    }

    public String overload() {
        return "no-args";
    }

    public String overload(String value) {
        return value;
    }

    private static String hiddenStatic() {
        return "hidden";
    }

    public static int answer() {
        return 42;
    }

    public static void resetVoidCounter() {
        voidCounter = 0;
    }

    public static int voidCounter() {
        return voidCounter;
    }

    public static void bumpVoidCounter() {
        voidCounter += 1;
    }

    public static boolean staticBoolean() {
        return true;
    }

    public static byte staticByte() {
        return 7;
    }

    public static char staticChar() {
        return 'A';
    }

    public static short staticShort() {
        return 1234;
    }

    public static long staticLong() {
        return 1234567890123L;
    }

    public static float staticFloat() {
        return 1.25f;
    }

    public static double staticDouble() {
        return 3.5d;
    }

    public static String staticString() {
        return "original-string";
    }

    public static int staticAdd(int left, int right) {
        return left + right;
    }

    public static int staticPrimitiveMix(boolean flag, byte value, char letter, short extra) {
        int total = value + letter + extra;
        return flag ? total : -total;
    }

    public static long staticWide(long value, double extra) {
        return value + (long) extra;
    }

    public static double staticFloatMix(float value, double extra) {
        return value + extra;
    }
}
