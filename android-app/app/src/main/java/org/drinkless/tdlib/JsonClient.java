// Copyright Aliaksei Levin, Arseny Smirnov 2014-2026.
// Boost Software License 1.0. Sourced from tdlib/td commit 022d602.
package org.drinkless.tdlib;

/** Official TDLib JSON JNI interface. */
public final class JsonClient {
  static { System.loadLibrary("tdjsonjava"); }
  public static native int createClientId();
  public static native void send(int clientId, String request);
  public static native String receive(double timeout);
  public static native String execute(String request);
  public interface LogMessageHandler { void onLogMessage(int verbosityLevel, String message); }
  public static native void setLogMessageHandler(int maxVerbosityLevel, LogMessageHandler handler);
  private JsonClient() {}
}
