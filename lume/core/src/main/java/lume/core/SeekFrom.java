package lume.core;

public sealed interface SeekFrom permits SeekFrom.Start, SeekFrom.Current, SeekFrom.End {
    final class Start implements SeekFrom {
        public static final Start INSTANCE = new Start();

        private Start() {}
    }

    final class Current implements SeekFrom {
        public static final Current INSTANCE = new Current();

        private Current() {}
    }

    final class End implements SeekFrom {
        public static final End INSTANCE = new End();

        private End() {}
    }
}
