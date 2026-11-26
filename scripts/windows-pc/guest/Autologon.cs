// The parts of automatic logon that PowerShell has no command for: the LSA
// secret Winlogon reads the password from, and a check of an account's
// password. Compiled by setup.ps1 and uninstall.ps1 with Add-Type.
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

namespace Wn13
{
    public static class Autologon
    {
        // Winlogon reads this secret before the registry's DefaultPassword.
        // Unlike that registry value, only administrators can read it.
        const string SecretName = "DefaultPassword";
        const uint POLICY_GET_PRIVATE_INFORMATION = 0x00000004;
        const uint POLICY_CREATE_SECRET = 0x00000020;
        const uint STATUS_OBJECT_NAME_NOT_FOUND = 0xC0000034;
        const int LOGON32_LOGON_NETWORK = 3;
        const int LOGON32_PROVIDER_DEFAULT = 0;
        const int ERROR_LOGON_FAILURE = 1326;
        // Refusals that mean the password itself was right: logon type not
        // granted (1385), must change (1907), expired (1330).
        static readonly int[] RightPasswordErrors = { 1385, 1907, 1330 };

        [StructLayout(LayoutKind.Sequential)]
        struct LSA_UNICODE_STRING
        {
            public ushort Length, MaximumLength;
            public IntPtr Buffer;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct LSA_OBJECT_ATTRIBUTES
        {
            public int Length;
            public IntPtr RootDirectory, ObjectName;
            public uint Attributes;
            public IntPtr SecurityDescriptor, SecurityQualityOfService;
        }

        [DllImport("advapi32.dll")]
        static extern uint LsaOpenPolicy(IntPtr system, ref LSA_OBJECT_ATTRIBUTES attributes, uint access, out IntPtr policy);

        [DllImport("advapi32.dll")]
        static extern uint LsaRetrievePrivateData(IntPtr policy, ref LSA_UNICODE_STRING key, out IntPtr data);

        [DllImport("advapi32.dll")]
        static extern uint LsaStorePrivateData(IntPtr policy, ref LSA_UNICODE_STRING key, IntPtr data);

        [DllImport("advapi32.dll")]
        static extern uint LsaFreeMemory(IntPtr buffer);

        [DllImport("advapi32.dll")]
        static extern uint LsaClose(IntPtr policy);

        [DllImport("advapi32.dll")]
        static extern int LsaNtStatusToWinError(uint status);

        [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        static extern bool LogonUser(string user, string domain, string password, int type, int provider, out IntPtr token);

        [DllImport("kernel32.dll")]
        static extern bool CloseHandle(IntPtr handle);

        static void Check(uint status, string what)
        {
            if (status != 0)
            {
                throw new Win32Exception(LsaNtStatusToWinError(status), what);
            }
        }

        // Runs body with the policy handle and an LSA string for the secret's
        // name, freeing both however it ends.
        static T WithSecretKey<T>(Func<IntPtr, LSA_UNICODE_STRING, T> body)
        {
            LSA_OBJECT_ATTRIBUTES attributes = new LSA_OBJECT_ATTRIBUTES();
            IntPtr policy;
            Check(LsaOpenPolicy(IntPtr.Zero, ref attributes, POLICY_GET_PRIVATE_INFORMATION | POLICY_CREATE_SECRET, out policy),
                "LsaOpenPolicy");
            LSA_UNICODE_STRING key = new LSA_UNICODE_STRING();
            key.Buffer = Marshal.StringToHGlobalUni(SecretName);
            key.Length = (ushort)(SecretName.Length * 2);
            key.MaximumLength = (ushort)(key.Length + 2);
            try
            {
                return body(policy, key);
            }
            finally
            {
                Marshal.FreeHGlobal(key.Buffer);
                LsaClose(policy);
            }
        }

        /// The password Winlogon would log on with, or null when it keeps none
        /// or an empty one.
        public static string ReadPassword()
        {
            return WithSecretKey<string>((policy, key) =>
            {
                IntPtr data;
                uint status = LsaRetrievePrivateData(policy, ref key, out data);
                if (status == STATUS_OBJECT_NAME_NOT_FOUND)
                {
                    return null;
                }
                Check(status, "LsaRetrievePrivateData");
                try
                {
                    if (data == IntPtr.Zero)
                    {
                        return null;
                    }
                    LSA_UNICODE_STRING value = (LSA_UNICODE_STRING)Marshal.PtrToStructure(data, typeof(LSA_UNICODE_STRING));
                    // Removing the secret leaves it empty, which logs nobody on.
                    return value.Length == 0 ? null : Marshal.PtrToStringUni(value.Buffer, value.Length / 2);
                }
                finally
                {
                    LsaFreeMemory(data);
                }
            });
        }

        /// Stores the password Winlogon logs on with; null removes it.
        public static void WritePassword(string password)
        {
            WithSecretKey<bool>((policy, key) =>
            {
                if (password == null)
                {
                    uint status = LsaStorePrivateData(policy, ref key, IntPtr.Zero);
                    if (status != STATUS_OBJECT_NAME_NOT_FOUND)
                    {
                        Check(status, "LsaStorePrivateData (remove)");
                    }
                    return true;
                }
                LSA_UNICODE_STRING value = new LSA_UNICODE_STRING();
                value.Buffer = Marshal.StringToHGlobalUni(password);
                value.Length = (ushort)(password.Length * 2);
                value.MaximumLength = (ushort)(value.Length + 2);
                IntPtr boxed = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(LSA_UNICODE_STRING)));
                try
                {
                    Marshal.StructureToPtr(value, boxed, false);
                    Check(LsaStorePrivateData(policy, ref key, boxed), "LsaStorePrivateData");
                }
                finally
                {
                    Marshal.FreeHGlobal(boxed);
                    Marshal.FreeHGlobal(value.Buffer);
                }
                return true;
            });
        }

        /// Whether password is the local account's: true when Windows takes
        /// it, even if the account may not log on this way or must change
        /// it; false only when Windows answers that it is wrong.
        public static bool IsPassword(string user, string password)
        {
            IntPtr token;
            if (LogonUser(user, ".", password, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, out token))
            {
                CloseHandle(token);
                return true;
            }
            int error = Marshal.GetLastWin32Error();
            if (error == ERROR_LOGON_FAILURE)
            {
                return false;
            }
            if (Array.IndexOf(RightPasswordErrors, error) >= 0)
            {
                return true;
            }
            throw new Win32Exception(error, "LogonUser");
        }
    }
}
