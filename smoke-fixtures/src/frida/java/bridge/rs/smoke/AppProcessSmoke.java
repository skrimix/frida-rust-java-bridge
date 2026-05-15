package frida.java.bridge.rs.smoke;

public final class AppProcessSmoke {
    private AppProcessSmoke() {
    }

    public static void main(String[] args) {
        System.load("/data/local/tmp/frida-java-bridge-rs/libfrida_java_bridge_rs.so");

        String result = nativeRun(AppProcessSmoke.class.getClassLoader());
        if (!"ok".equals(result)) {
            throw new RuntimeException(result);
        }

        System.out.println("app_process_smoke: ok");
        Runtime.getRuntime().halt(0);
    }

    private static native String nativeRun(ClassLoader loader);
}
