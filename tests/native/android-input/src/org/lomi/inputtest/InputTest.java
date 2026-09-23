package org.lomi.inputtest;

import android.app.Activity;
import android.os.Bundle;
import android.graphics.Canvas;
import android.graphics.Color;
import android.view.MotionEvent;
import android.view.View;
import android.widget.EditText;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Disposable guest fixture for text assertions and measured touch-to-image changes. */
public final class InputTest extends Activity {
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        android.util.Log.i("LomiInputTest", "MCP fixture started: Zażółć 🙂");
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(0, 100, 0, 0);
        TextView label = new TextView(this);
        label.setText("Native Unicode, composition and touch latency test");
        layout.addView(label);
        EditText editor = new EditText(this);
        editor.setSingleLine(false);
        editor.setContentDescription("lomi-test-editor");
        layout.addView(editor, new LinearLayout.LayoutParams(-1, 300));
        TextView submitted = new TextView(this);
        submitted.setContentDescription("lomi-test-result");
        Button submit = new Button(this);
        submit.setText("Submit Unicode form");
        submit.setContentDescription("lomi-test-submit");
        submit.setOnClickListener(view -> {
            String value = editor.getText().toString();
            submitted.setText("Submitted: " + value);
            android.util.Log.i("LomiInputTest", "MCP form submitted: " + value);
        });
        layout.addView(submit, new LinearLayout.LayoutParams(-1, -2));
        layout.addView(submitted, new LinearLayout.LayoutParams(-1, -2));
        View target = new View(this) {
            private boolean on;
            @Override protected void onDraw(Canvas canvas) {
                canvas.drawColor(on ? Color.WHITE : Color.BLACK);
            }
            @Override public boolean onTouchEvent(MotionEvent event) {
                label.setText("Touch " + Math.round(event.getRawX()) + "," + Math.round(event.getRawY())
                    + " action " + event.getActionMasked());
                if (event.getAction() == MotionEvent.ACTION_DOWN) {
                    on = !on;
                    invalidate();
                }
                return true;
            }
        };
        layout.addView(target, new LinearLayout.LayoutParams(-1, 0, 1));
        setContentView(layout);
        editor.requestFocus();
    }
}
