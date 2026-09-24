export interface ControlStartupState {
  supported: boolean;
  autoStart: boolean | null;
  yoloMode: boolean;
  error: string | null;
}
