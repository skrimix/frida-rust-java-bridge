package frida.java.bridge.rs.test;

public final class TestSubject {
    public static final String STATIC_TEXT = "static-test";
    private static int voidCounter = 0;
    public int number = 7;
    public int instanceVoidCounter = 0;
    private long hidden = 11L;

    public TestSubject() {
    }

    public TestSubject(int number) {
        this.number = number;
    }

    public String message() {
        return "dex-test";
    }

    public int instanceNumber() {
        return number;
    }

    public int facadeInstanceNumber() {
        return number + 100;
    }

    public int lifecycleInstanceNumber() {
        return number + 700;
    }

    public int facadeLifecycleInstanceNumber() {
        return number + 710;
    }

    public void bumpInstanceVoidCounter() {
        instanceVoidCounter += 1;
    }

    public void objectSink(Object value) {
        instanceVoidCounter += value == null ? 20 : 10;
    }

    public int instanceVoidCounter() {
        return instanceVoidCounter;
    }

    public boolean instanceBoolean() {
        return (number & 1) != 0;
    }

    public byte instanceByte() {
        return (byte) (number - 24);
    }

    public char instanceChar() {
        return 'A';
    }

    public short instanceShort() {
        return (short) (number + 1203);
    }

    public long instanceLong() {
        return 1234567890123L + number;
    }

    public float instanceFloat() {
        return number + 0.25f;
    }

    public double instanceDouble() {
        return number + 0.5d;
    }

    public int instanceAdd(int left, int right) {
        return number + left + right;
    }

    public int instancePrimitiveMix(boolean flag, byte value, char letter, short extra) {
        int total = number + value + letter + extra;
        return flag ? total : -total;
    }

    public long instanceWide(long value, double extra) {
        return number + value + (long) extra;
    }

    public double instanceFloatMix(float value, double extra) {
        return number + value + extra;
    }

    public String overload() {
        return "no-args";
    }

    public String overload(String value) {
        return value;
    }

    public String facadeOverload(String value) {
        return value;
    }

    public Object objectEcho(Object value) {
        return value;
    }

    public Object[] objectArrayEcho(Object[] value) {
        return value;
    }

    public int[] intArrayEcho(int[] value) {
        return value;
    }

    public int sumIntArray(int[] value) {
        int total = 0;
        for (int item : value) {
            total += item;
        }
        return total;
    }

    public TestSubject subjectEcho(TestSubject value) {
        return value;
    }

    private static String hiddenStatic() {
        return "hidden";
    }

    public static int answer() {
        return 42;
    }

    public static int facadeAnswer() {
        return 314;
    }

    public static int lifecycleStaticAnswer() {
        return 700;
    }

    public static int facadeLifecycleAnswer() {
        return 710;
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

    public static void staticObjectSink(Object value) {
        voidCounter += value == null ? 20 : 10;
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

    public static String staticEcho(String value) {
        return value;
    }

    public static Object staticObjectEcho(Object value) {
        return value;
    }

    public static Object facadeStaticObjectEcho(Object value) {
        return value;
    }

    public static Object staticObjectPairEcho(Object first, Object second) {
        return first != null ? first : second;
    }

    public static Object[] staticObjectArrayEcho(Object[] value) {
        return value;
    }

    public static int[] staticIntArrayEcho(int[] value) {
        return value;
    }

    public static boolean[] staticBooleanArrayEcho(boolean[] value) {
        return value;
    }

    public static Object[] facadeStaticObjectArrayEcho(Object[] value) {
        return value;
    }

    public static TestSubject staticSubjectEcho(TestSubject value) {
        return value;
    }

    public Object startupLoadedApkSix(Object first, Object second, Object third, boolean fourth, boolean fifth, boolean sixth) {
        return first;
    }

    public Object startupLoadedApkSeven(Object first, Object second, Object third, boolean fourth, boolean fifth, boolean sixth, boolean seventh) {
        return first;
    }

    public Object startupLoadedApkThree(Object first, Object second, int third) {
        return first;
    }

    public Object startupLoadedApkString(String first, Object second, int third) {
        return first;
    }

    public Object startupMakeApplication(boolean forceDefaultAppClass, Object instrumentation) {
        return instrumentation;
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
