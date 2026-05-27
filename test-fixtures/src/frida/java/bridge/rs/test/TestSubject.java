package frida.java.bridge.rs.test;

public final class TestSubject extends TestSubjectBase {
    public static final String STATIC_TEXT = "static-test";
    private static int voidCounter = 0;
    public static boolean staticFlag = true;
    public static byte staticSmall = 2;
    public static char staticLetter = 'C';
    public static short staticShortNumber = 123;
    public static long staticWideNumber = 1000L;
    public static float staticRatio = 1.5f;
    public static double staticPrecise = 2.5d;
    public static int shadowedNumber = 29;
    public int number = 7;
    public int shadowedStaticField = 73;
    public boolean flag = true;
    public byte small = 2;
    public char letter = 'C';
    public short shortNumber = 123;
    public long wideNumber = 1000L;
    public float ratio = 1.5f;
    public double precise = 2.5d;
    public int instanceVoidCounter = 0;
    public TestSubject subjectValue = null;
    private long hidden = 11L;

    public TestSubject() {
    }

    public TestSubject(int number) {
        this.number = number;
    }

    public String message() {
        return "dex-test";
    }

    public String shadowedMessage() {
        return "child-shadowed";
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

    public double instanceStackSpill(
            int first,
            int second,
            int third,
            int fourth,
            int fifth,
            int sixth,
            int seventh,
            int eighth,
            double ninth,
            double tenth,
            double eleventh,
            double twelfth,
            double thirteenth,
            double fourteenth,
            double fifteenth,
            double sixteenth,
            double seventeenth) {
        return number
                + first
                + second
                + third
                + fourth
                + fifth
                + sixth
                + seventh
                + eighth
                + ninth
                + tenth
                + eleventh
                + twelfth
                + thirteenth
                + fourteenth
                + fifteenth
                + sixteenth
                + seventeenth;
    }

    public String overload() {
        return "no-args";
    }

    public String overload(String value) {
        return value;
    }

    public String overload(Object value) {
        return value == null ? "object-null" : value.toString();
    }

    public String facadeOverload(String value) {
        return value;
    }

    public Object objectEcho(Object value) {
        return value;
    }

    public Object objectPairEcho(Object first, Object second) {
        return first != null ? first : second;
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

    public static int facadeThrowingAnswer() {
        throw new IllegalStateException("facade-boom");
    }

    public int facadeThrowingInstanceNumber() {
        throw new IllegalStateException("facade-instance-boom");
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

    public static CharSequence staticCharSequence() {
        return "original-char-sequence";
    }

    public static String staticEcho(String value) {
        return value;
    }

    public static Object staticObjectEcho(Object value) {
        return value;
    }

    public static String staticCharSequenceEcho(CharSequence value) {
        return value == null ? null : value.toString();
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

    public static int staticIdentity(int value) {
        return value;
    }

    public static boolean staticBooleanFromInt(int value) {
        return value > 0;
    }

    public static byte staticByteFromByte(byte value) {
        return (byte) (value + 1);
    }

    public static char staticCharFromChar(char value) {
        return (char) (value + 1);
    }

    public static short staticShortFromShort(short value) {
        return (short) (value + 1);
    }

    public static float staticFloatFromFloat(float value) {
        return value + 1.5f;
    }

    public static void staticObjectIntSink(Object value, int extra) {
        voidCounter += (value == null ? 20 : 10) + extra;
    }

    public static Object staticReferencePrimitiveArrayMix(Object first, int value, Object[] second, boolean chooseArray) {
        if (chooseArray) {
            return second;
        }
        return value > 0 ? first : null;
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

    public static double staticStackSpill(
            int first,
            int second,
            int third,
            int fourth,
            int fifth,
            int sixth,
            int seventh,
            int eighth,
            double ninth,
            double tenth,
            double eleventh,
            double twelfth,
            double thirteenth,
            double fourteenth,
            double fifteenth,
            double sixteenth,
            double seventeenth) {
        return first
                + second
                + third
                + fourth
                + fifth
                + sixth
                + seventh
                + eighth
                + ninth
                + tenth
                + eleventh
                + twelfth
                + thirteenth
                + fourteenth
                + fifteenth
                + sixteenth
                + seventeenth;
    }
}
