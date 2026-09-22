package org.lomi.input;

import android.inputmethodservice.InputMethodService;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import org.json.JSONObject;

/** Explicitly selected guest IME; all text stays inside the managed device. */
public final class LomiInput extends InputMethodService {
    private final Handler main = new Handler(Looper.getMainLooper());
    private volatile boolean closed;
    private volatile LocalServerSocket server;
    private volatile LocalSocket client;
    private boolean editing;

    @Override public void onCreate() {
        super.onCreate();
        Thread worker = new Thread(this::serve, "lomi-input");
        worker.setDaemon(true);
        worker.start();
    }

    @Override public boolean onEvaluateInputViewShown() { return false; }

    @Override public void onStartInput(EditorInfo info, boolean restarting) {
        super.onStartInput(info, restarting);
        editing = true;
    }

    @Override public void onFinishInput() {
        editing = false;
        super.onFinishInput();
    }

    @Override public void onDestroy() {
        closed = true;
        try { if (client != null) client.close(); } catch (IOException ignored) { }
        try { if (server != null) server.close(); } catch (IOException ignored) { }
        super.onDestroy();
    }

    private void serve() {
        while (!closed) {
            try (LocalServerSocket listener = new LocalServerSocket("lomi.input.v1")) {
                server = listener;
                while (!closed) {
                    try (LocalSocket socket = listener.accept()) {
                        client = socket;
                        int uid = socket.getPeerCredentials().getUid();
                        // adbd forwards as shell (production images) or root (AOSP images).
                        if (uid != 2000 && uid != 0) continue;
                        socket.setSoTimeout(5000);
                        DataInputStream in = new DataInputStream(socket.getInputStream());
                        DataOutputStream out = new DataOutputStream(socket.getOutputStream());
                        while (!closed) {
                            int length = in.readInt();
                            if (length <= 0 || length > 65536) break;
                            byte[] bytes = new byte[length];
                            in.readFully(bytes);
                            String json = StandardCharsets.UTF_8.newDecoder()
                                .onMalformedInput(CodingErrorAction.REPORT)
                                .onUnmappableCharacter(CodingErrorAction.REPORT)
                                .decode(ByteBuffer.wrap(bytes)).toString();
                            JSONObject request = new JSONObject(json);
                            JSONObject[] response = new JSONObject[1];
                            CountDownLatch done = new CountDownLatch(1);
                            Runnable apply = () -> {
                                response[0] = dispatch(request);
                                done.countDown();
                            };
                            main.post(apply);
                            if (!done.await(2, TimeUnit.SECONDS)) {
                                main.removeCallbacks(apply);
                                break;
                            }
                            byte[] reply = response[0].toString().getBytes(StandardCharsets.UTF_8);
                            out.writeInt(reply.length);
                            out.write(reply);
                            out.flush();
                        }
                    } catch (Exception ignored) {
                        // A disconnected or malformed client cannot retain the only connection.
                    } finally { client = null; }
                }
            } catch (IOException ignored) {
                // Android can create the replacement IME before destroying its old
                // service. Retry the private bind after the old listener is released.
                if (!closed) {
                    try { Thread.sleep(100); }
                    catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); return; }
                }
            } finally { server = null; }
        }
    }

    private JSONObject dispatch(JSONObject request) {
        JSONObject reply = new JSONObject();
        try {
            reply.put("version", 1);
            reply.put("id", request.getLong("id"));
            if (request.getInt("version") != 1) throw new IllegalArgumentException("Unsupported protocol");
            String device = Settings.Global.getString(getContentResolver(), "lomi_device");
            String generation = Settings.Global.getString(getContentResolver(), "lomi_generation");
            if (device == null || generation == null || !device.equals(request.getString("deviceId"))
                    || !generation.equals(request.getString("generationKey"))) {
                throw new IllegalStateException("This transport belongs to a different Android instance");
            }
            String action = request.getString("action");
            if (action.equals("ping")) {
                reply.put("ok", true);
                return reply;
            }
            InputConnection connection = getCurrentInputConnection();
            if (!editing || connection == null) throw new IllegalStateException("Focus an editable Android field");
            boolean accepted;
            switch (action) {
                case "commit": accepted = connection.commitText(request.getString("text"), 1); break;
                case "compose": accepted = connection.setComposingText(request.getString("text"), 1); break;
                case "finish": accepted = connection.finishComposingText(); break;
                case "delete": accepted = connection.deleteSurroundingTextInCodePoints(1, 0); break;
                case "paste":
                    ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
                    clipboard.setPrimaryClip(ClipData.newPlainText("Lomi", request.getString("text")));
                    accepted = connection.performContextMenuAction(android.R.id.paste);
                    break;
                default: throw new IllegalArgumentException("Unknown input operation");
            }
            if (!accepted) throw new IllegalStateException("Editor rejected input");
            reply.put("ok", true);
        } catch (Exception error) {
            try { reply.put("ok", false); reply.put("error", error.getMessage()); }
            catch (Exception ignored) { }
        }
        return reply;
    }
}
