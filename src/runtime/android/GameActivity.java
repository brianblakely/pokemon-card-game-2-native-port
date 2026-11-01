package io.github.pokemon_card_game_2_native_port;

import org.libsdl.app.SDLActivity;

/**
 * SDL's activity, which loads libSDL3.so and the app (libmain.so) and runs
 * its SDL_main. The app takes the desktop's command line from the intent's
 * "args" extra, for tests:
 * {@code am start -n PACKAGE/.GameActivity --esa args --exit-after-frames,600,--stats}
 */
public class GameActivity extends SDLActivity {
    @Override
    protected String[] getArguments() {
        String[] arguments = getIntent().getStringArrayExtra("args");
        return arguments != null ? arguments : new String[0];
    }
}
