#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>

// GPUI owns the application delegate. Observe native launch and add the action
// once GPUI has installed the application menu. CLI version probes never launch AppKit.
@interface PrimeAgentUpdates : NSObject
@end

@implementation PrimeAgentUpdates
static SPUStandardUpdaterController *updater;

+ (void)load {
    [[NSNotificationCenter defaultCenter] addObserver:self
        selector:@selector(applicationLaunched:) name:NSApplicationDidFinishLaunchingNotification object:nil];
}

+ (void)applicationLaunched:(NSNotification *)notification {
    (void)notification;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (updater || ![NSBundle.mainBundle objectForInfoDictionaryKey:@"SUFeedURL"]) {
            return;
        }
        updater = [[SPUStandardUpdaterController alloc] initWithStartingUpdater:YES
            updaterDelegate:nil userDriverDelegate:nil];
        NSMenu *menu = NSApp.mainMenu.itemArray.firstObject.submenu;
        NSMenuItem *check = [[NSMenuItem alloc] initWithTitle:@"Check for Updates…"
            action:@selector(checkForUpdates:) keyEquivalent:@""];
        check.target = updater;
        [menu insertItem:check atIndex:MIN(1, menu.numberOfItems)];
    });
}
@end
