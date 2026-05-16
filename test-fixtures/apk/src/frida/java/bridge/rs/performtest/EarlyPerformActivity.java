package frida.java.bridge.rs.performtest;

import android.app.Activity;
import android.os.Bundle;
import android.widget.TextView;

public final class EarlyPerformActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        TextView view = new TextView(this);
        view.setText("frida-java-bridge-rs APK perform test");
        setContentView(view);
    }
}
