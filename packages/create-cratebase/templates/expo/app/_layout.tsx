import { Stack } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { CratebaseProvider } from "@cratebase/react";
import { cb } from "@/lib/cratebase";
import { useTheme } from "@/lib/theme";

export default function RootLayout() {
  const theme = useTheme();
  return (
    <SafeAreaProvider>
      <CratebaseProvider client={cb}>
        <StatusBar style="auto" />
        <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: theme.background } }} />
      </CratebaseProvider>
    </SafeAreaProvider>
  );
}
